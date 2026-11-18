# egui 0.37 migration plan for craft-libs

Status update (2026-10-25): local candidate includes main `865c9af7fdf455cd3892402d18e526a9f019c6bc`, preserving current native docking, blocked-window focus and print-copy handling. Native input-callback cancellation no longer re-enters the plugin mutex. Both cancellation regressions reproduced the deadlock before the fix. Current gates passed: 1,774 test executions, zero failures, 21 ignored, plus strict workspace/UI/print lint, UI all-feature tests, native example build and UI WASM check. Independent merge/implementation and compiler-correction reviews passed. App-pinned ancestors 25da1314 and 650b1c0 remain reachable. Current-source native platforms/GUI/printer hardware and paired performance remain open; the attempted paired run stopped before measurement due to a collector metadata-projection bug. Publication needs separate explicit approval; human review and human merge are required.

The original planning record below is historical.

Status: final landing integration, 2026-10-25. The reviewed shared UI includes screenshot completion, shaped-text alignment, keymap support and regression coverage. The manifest and lock select egui `6d518fc799681571963842f95b55a6b68431b15a` and winit `a517b3846ee8457d84ee4975110bfda6797c44dc` through canonical GitHub URLs.

The integration of migration ancestor `25da1314d443171b4adeaad2cd9454b273f35c6f` with main `2ab6049d9031bc8fae7f7df669b5fccaf5b9966f` passed formatting, strict workspace all-target clippy, workspace tests and the PDF all-feature suite in managed job `egui037-craft-libs-current-main-gates-20261024-2021`. Source and lock hashes were checked against that job before the local integration checkpoint.

New main `6e7f28518dd1cb57e1624bdc269c3c14316b13ae` adds docking, native floating windows and other shared-library changes. Its integration passed fresh review. That review identified an IME reset acknowledgement lost when a native floating window was blocked; the fix preserves ordered control messages in both raw and processed panel input, with a native-driver regression. A second review accepted the fix.

Managed job `egui037-craft-libs-native-fix-gates-20261025-0328` passed formatting, strict workspace lint, workspace tests, UI all-feature lint/tests, the native docking example build, UI WASM check, and prepress/plate-test all-feature lint/tests with hash-verified licensed PDF test fonts. Its 121 test summaries total 1,723 passed, 0 failed and 25 ignored; overlapping feature suites are included in those totals. PDF all-feature tests also passed in job `egui037-craft-libs-new-main-ci-20261025-0242`. Further shared-crate feature/platform matrices, visible native-window workflows and final performance acceptance remain open. App-pinned migration ancestors remain reachable. The prepared benchmark protocol has no accepted timing result; the completed egui text benchmark remains separately inconclusive. Earlier test counts below are historical snapshots.

Every push or new PR requires approval of its exact commit range and validation. Human review and merge are required. The baseline tables below preserve the original planning record.

This plan covers only `storytold/craft-libs`. It records the shared-library work needed before the
Craft applications move to the egui 0.37 family. Application-specific UI migrations, their document
models, command routing, persistence policy and native window ownership remain in those applications.

The [suite rollout plan](egui-037-suite-rollout.md) supplies the shared publication order,
measurement protocol and cross-application acceptance contract. Stricter repository gates apply.

## 1. Baseline and provenance

### 1.1 Exact source baseline

| Item | Revision / value |
| --- | --- |
| craft-libs checkout | `49190884278f16c9a9f5823d7698a8ffb5deb062` (`Merge pull request #238 ...`, 2026-10-22) |
| older local comparison point | `b1401e3` (stale local `origin/main`; not the current remote baseline) |
| lockfile | `Cargo.lock`, format version 4 |
| current egui family | `egui`, `epaint`, `ecolor`, `emath`, `eframe`, `egui-wgpu`, `egui-winit`, `egui_extras`, `egui_kittest` all `0.36.2` |
| current winit | `0.30.13`, crates.io checksum `a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d` |
| current accesskit | `0.24.1`; `accesskit_consumer` `0.35.0`; `kittest` `0.4.0` |
| target upstream release | egui annotated tag `0.37.0`, peeled commit `841a328ba2dd8f6996beb7189ebb7fc00e668a2a`, released 2026-10-15; **not** `main` |
| egui source candidate | `aiongg/egui` `craft/0.37` at `a4f9a17f64a6bae61350fb85804c9df0d9865916`; integrate/review into storytold before production pinning |
| winit source candidate | `aiongg/winit` `craft/0.30.13-lang` at `8b35388bd719e8cc86a95971ba0bc026c9a8f058`; integrate/review into storytold before production pinning |
| storytold winit base | `craft/0.30.13` at `ffae5198770d7730e718043f035a772b7dbc3b1b`, itself based on winit tag `v0.30.13` (`e9809ef5`) |
| craft-libs PR under review | `storytold/craft-libs` PR 199, OPEN / CONFLICTING at head `b68de8c5f4f1c060904d211a7b1320aa0f7bce78`; its merge into local audit branch is `ec8840af8c5fda695cf195cdd05e5ae078415c07` |

The audit merge `ec8840a` is **not** a usable target: its conflicted `Cargo.lock` still resolves egui
0.36.2 even though the manifest and source use the 0.37 API. The PR branch itself has a 0.37.0
registry lock. The historical audit log at
`/home/codex/work/ssd2/task-evidence/egui037-audit-20261022/fork/craft-libs.log` shows a successful
`cargo check` after the audit harness patched 11 crates to the fork path; it is a useful lead, not
fresh evidence and not a test run. Fresh validation uses `maintenance/task.py run/status/finish`: retain the owning resource, source
and evidence paths across sequential commands and use a fresh job ID for each intentional command.

### 1.2 Dependents and feature targets in this workspace

The root workspace has no application binaries or examples. Excluded vendored PDF
crates retain their own examples/workspaces; keep them unchanged. Relevant root targets are
libraries and their tests:

| Crate | egui use | Features / targets |
| --- | --- | --- |
| `craft-ui` (`crates/ui`) | public components and tests | production `egui` with `default-features = false`; optional `serde`; dev `egui` with `default_fonts`, `egui_extras/svg`, `egui_kittest/wgpu`, `image/png` |
| `craft-window` (`crates/window`) | optional `decorate(egui::ViewportBuilder)` | `egui` feature; Wayland probe only on Linux/BSD; pure logic without egui |
| `craft-quit` (`crates/quit`) | optional close handling and app menu | `egui`; `menu` implies `egui`; menu/terminate code only on macOS |
| `craft-platform` (`crates/platform`) | optional deferred close and native theme hook | `egui` with `dialogs` and/or `appearance`; `appearance` only on unix non-macOS non-wasm; `browser` only on wasm |
| other crates | none, except references in documentation | no egui dependency |

`Cargo.toml:34` is the workspace pin; `crates/ui/Cargo.toml:20-24` is the only egui test toolchain.
`docs/ui-components-audit.md:5` and `crates/ui/README.md:237` still say 0.36; both need updating after
validation, but historical source citations in the audit must not be silently rewritten.

### 1.3 Current tests and CI

Source-declared `#[test]` occurrences at the baseline: 728 workspace-wide, 91 in `crates/ui`
(85 in `src`, 6 integration). These counts are not proof of execution. The UI CI matrix is Ubuntu,
Windows and macOS for `cargo fmt`, clippy, `cargo test -p craft-ui` (default and all features), plus a
wasm32 `cargo check` (`.github/workflows/ui.yml:20-58`). The visual suite declares 372 control pairs,
36 deliberate negative pairs, 18 toolbar pairs, 12 palette pairs and 12 SVG pairs; every positive
pair requires exact RGBA and mesh equality (`crates/ui/tests/visual_regression.rs:185-394`).

The repository AGENTS.md additionally requires before merge:
`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo test --workspace`. Those are the minimum gates; the package-specific CI matrix and the fork
contract below add coverage rather than replacing them.

## 2. What actually changes in egui 0.37

The following are released 0.37 behavior, not fork behavior. They are the only reasons `craft-ui`
needs production edits for the upstream release.

| Change | Effect in craft-libs | Source anchor |
| --- | --- | --- |
| `WidgetType` is replaced by `accesskit::Role` (`egui::Role`); compatibility alias is deprecated | Every `WidgetInfo` constructor must use `Role`; `SelectableLabel` becomes `Button`, `Other` becomes `Unknown`, `Checkbox` becomes `CheckBox` | `egui/src/lib.rs` 0.37.0 lines ~640-655; PR #8555 |
| `Id::new` and `AsId` are deprecated; `Id::unique`, `IdSalt`, `IdSaltSet/Map` are the explicit APIs | Test IDs and any caller-provided IDs should use `Id::unique`; use `IdSalt` for scope-local salts | `egui/src/id.rs`; PRs #8534, #8586, #8590 |
| `Ui::id` is renamed `Ui::scope_id`; `UiBuilder::id` is renamed `scope_id` | Child body/header UIs must pass a globally unique scope id; tests and README examples must use `scope_id` | `egui/src/ui.rs` 0.37.0 lines ~359-383; `ui_builder.rs` lines ~63-82; PR #8542 |
| `Panel` constructors accept `impl AsIdSalt`; `CentralPanel` stays last; root `Ui` is the normal app surface | Application panel code must keep panel order and use a stable salt; craft-ui's `DockArea` is a child of that root/panel UI, not a `Context` replacement | `egui/src/containers/panel.rs` 0.37.0; PR #8592; `Context::run_ui` / `root_ui` / `run_pass` |
| `default_fonts` is no longer a default feature; eframe still enables it by default, but integrations must choose | `craft-ui` production already disables defaults; test dev-dependency must keep `default_fonts`; applications with custom fonts must verify coverage | `egui/Cargo.toml` 0.37.0; PR #8583 |
| egui 0.37 adds missing-glyph policy and system font providers; kittest panics on missing glyph by default | Tests that deliberately draw arbitrary script text must opt into `MissingGlyphPolicy::Tofu`; production should keep tofu rather than panic; custom app fonts need explicit coverage checks | `epaint/src/text/fonts.rs`; `egui_kittest/src/builder.rs` (`missing_glyph_policy`); PRs #8601, #8512 |
| `ViewportCommand::Screenshot` takes a one-shot `ScreenshotCallback`; `Context::request_screenshot` is the convenience API | Any app screenshot path, not craft-ui itself, must move from `UserData`/`Event::Screenshot` correlation to callbacks | `egui/src/data/screenshot_callback.rs`; `viewport.rs:1202-1205`; PR #8489 |
| `Response::accessible_name`, `Ui::label_inputs_by` / `name_inputs`, `Area::role`, better popup/menu trees | Accessibility labels/roles must be explicit for input widgets and icon-only controls; kittest now fails unnamed inputs | PRs #8604, #8608, #8612, #8581, #8589 |
| Style defaults changed (roomier buttons, larger corner radius); `Style` theme plugin is experimental | `DockStyle::from_ui` inherits changed defaults; visual fixtures must keep explicit geometry/colors instead of accepting a new golden | PRs #8617, #8445, #8153 |
| `Context::run_ui`, `run_pass`, `root_ui` and `Context::run_logic` define root-UI and hidden-window semantics | Tests must use the correct entry point; logic-only work must not require a painted `Ui`; `request_discard` may call a UI closure more than once | `egui/src/context.rs` 0.37.0; PR #8535; eframe `App::logic`/`App::ui` |
| PRIMARY selection support was added upstream | App clipboard behavior changes; E4 changes the backend on Wayland; PRIMARY must be tested separately from the regular clipboard | PR #8428; `egui-winit/src/clipboard.rs` |
| `egui_extras` and `egui_kittest` have their own 0.37 releases and features; `egui_plot` is not part of this repo | Do not assume every egui-family crate shares one version number; pin what is used and verify its manifest | crate manifests at tag `0.37.0` |

The upstream 0.37.0 tag itself has no `PATCHES.md`; fork-only material is in the suite branch. The
0.37.0 tag and the fork revision must never be confused: a registry-only build gets upstream
behavior, while a suite build gets E1-E21 and the winit patches.

## 3. Fork and winit provenance

craft-libs #198 is a broader language-support issue/proposal, not a PR or merged
dependency. It does not authorize a wholesale locale/font roadmap in this port.
PR 199 is open/conflicting at the recorded head; its stale merge tree must not
replace current main. Current upstream report status is dated research, not proof
of fixed behavior or fresh task validation.


### 3.1 The suite fork

`aiongg/egui` `craft/0.37` is based on tag `0.37.0` and merges the `craft/*` topic branches in the
order listed in its `PATCHES.md`. The final inspected revision is
`a4f9a17f64a6bae61350fb85804c9df0d9865916`. The branch and its `origin/ai-0.37` alias are movable;
all manifests and CI must use the full SHA. The older `origin/craft/0.37.0-trial`
(`59163e291c56ebe0199e191c23bf58ed157c9862`) is not the target because it lacks later language
work and is not the published suite reference.

The fork's workspace manifest pins winit by **branch**:

```toml
[patch.crates-io]
winit = { git = "https://github.com/aiongg/winit", branch = "craft/0.30.13-lang" }
```

That patch belongs to the egui workspace. Cargo does **not** propagate a dependency's patch section
into a consumer root. A consumer that only depends on `aiongg/egui` will resolve registry winit, and
E4/E19/E21 will fail to compile or silently use the wrong API. `craft-libs` and every app must add its
own `winit` patch with the immutable commit.

### 3.2 Winit patch ledger

Base: winit `v0.30.13` (`e9809ef5`), then storytold/winit `craft/0.30.13` (`ffae5198`) with W1-W15,
then `aiongg/winit` `craft/0.30.13-lang` (`8b35388b`) with W17-W27. W16 (Wayland pinch/rotation) is
not in this cut.

| Patch | Commit(s) | Behavior relevant to this plan | Removal / test condition |
| --- | --- | --- | --- |
| W1 | `b357a332` | Windows mixed-DPI `WM_DPICHANGED` rect | Remove when a winit release in use has #4341; verify DPI geometry |
| W2 | `13ca693b` | Wayland text input dies only with its seat | Remove with upstream #4747; test seat removal |
| W3 | `fd104a4c` | Windows IME caret from `GCS_CURSORPOS` only when reported | Test Korean caret; remove on release |
| W4 | `3fffb7ce` | Hidden maximized windows stay hidden | Test restore; remove on release |
| W5 | `283370ce` | X11 focus replacement IME context | Superseded by W20; remove together |
| W6 | `a82b90fb` | macOS NSTextInputClient commit flow | Test composition; remove on release |
| W7 | `9b90d5bd` | macOS discard marked text when IME disabled | Test disable; remove on release |
| W8 | `6a87addd` | macOS emoji picker text as commit | Test picker; remove on release |
| W9 | `9500f25d` | macOS no duplicate Korean space | Test Korean space; remove on upstream fix |
| W10 | `4a7dd791` | macOS first Apple Korean key retry | Test first key; remove on release |
| W11 | `4df8554b` | X11 scale factor from server DPI before physical DPI | Test mixed-DPI X11; remove when upstream equivalent lands |
| W12 | `7facf001` | macOS title-bar drag reaches app | Test drag; remove when upstream equivalent lands |
| W13 | `5506b1a9` | Wayland pixel wheel/tilt/continuous reports `Moved` | Test Ctrl zoom not latched; remove on upstream fix |
| W14 | `c488d2e9` | macOS opt-in cancelable termination -> close request | Test close/quit; keep until upstream release |
| W15 | `cf73a29c`, `b05ce129` | Wayland file drops and clipboard on one per-seat data device | Required by E4; remove only with a winit release carrying #4729/#4571 and equivalent regular-clipboard/data-device tests |
| W15P | New; no commit yet | Per-seat SCTK PRIMARY on the existing Wayland connection/event queue; `platform/wayland.rs`, `wayland/clipboard.rs`, `wayland/state.rs` and seat handlers | Require bounded asynchronous read/write transfers, total deadlines, byte/request/worker caps, cancellation and focused-owner completion; pointer/keyboard triggering serials, seats/offers, empty/absent/self-paste tests on wlroots/KWin/Mutter. Remove only with an equivalent released native path |
| W17 | `2be27382` | Regression guards for W2/W3/W5-W10/W12/W13 | Keep as tests while their patches remain |
| W18 | `894bf2d7` | Windows hidden system caret at IME area | Test Sogou/Japanese IME; remove on upstream fix |
| W19 | `fa2e1b3d` | Windows `VK_PROCESSKEY` not sent as a key press | Test IME key ownership; remove on upstream fix |
| W20 | `019c95f1` | X11 one input context per window; disable resets | Test Vietnamese Telex; replaces W5, remove together |
| W21 | `23bb19a7` | X11 IME spot after 100 ms quiet | Test forwarded keys not refiltered; remove on upstream fix |
| W22 | `0816871e` | macOS caret, replacement ranges, un-composed commits | Test press-and-hold/dictation; remove on release |
| W23 | `c8cd8aa1` | macOS layout's Command table for shortcut keys | Test Dvorak-Command/V; remove on upstream fix |
| W24 | `4b5bc5a0` | `Window::reset_ime()` for all four desktop backends | Required by IME reset tests; remove on release with equivalent |
| W25 | `eb4007da` | X11/Wayland `KeyEventExtLatinLayout` | Required by E19; remove on release with equivalent |
| W26 | `2b9cd1cf` | macOS IME delete-surrounding by UTF-8 bytes | Required by E21; remove after the egui -> winit 0.31 move |
| W27 | `f40c76a2` | Wayland text-input-v3 surrounding text and delete-surrounding | Required by E21; remove after the egui -> winit 0.31 move |

Provenance concerns:

- Integrate and review W17-W27 plus W15P and the W27 lifecycle revision in `storytold/winit` before pinning production consumers. Retain
  aiongg provenance; use the resulting full storytold commit, never a moving branch.
- W15's Wayland clipboard API and E4 are a pair. Do not take one without the other.
- W26/W27 use a fork-only delete-surrounding API shape. When winit gains core
  `Ime::DeleteSurrounding`, E21 and the winit patches must be removed together; otherwise the
  compiler sees two incompatible IME APIs.
- W11 changes `pixels_per_point`/DPR input. It affects tab fit, visual fixtures, screenshot pixel
  sizes and any performance comparison. Record the exact winit commit in those runs.

## 4. Audit of craft-libs PR 199

PR 199 is the useful starting point, not a branch to merge blindly.

### 4.1 What it gets right

- `Cargo.toml:34` changes the workspace egui pin to `0.37` and `crates/ui/Cargo.toml:23-24` changes
  `egui_extras` and `egui_kittest` to `0.37`.
- `crates/ui/src/buttons/mod.rs`, `crates/ui/src/docking/surface.rs`,
  `crates/ui/src/layout/resize.rs`, and `crates/ui/src/tabs/mod.rs` map the deprecated
  `WidgetType` values to the 0.37 roles: `SelectableLabel` -> `Button`, `Checkbox` -> `CheckBox`,
  `Other` -> `Unknown`. This matches egui's own mapping table.
- Test IDs move from `Id::new` to `Id::unique`; child-body IDs move from `Ui::id`/`UiBuilder::id` to
  `scope_id`. These are semantic renames, not a new ID scheme.
- `crates/ui/src/docking/tests.rs` replaces the `⋮` fixture label with `…`. A read-only cmap check of the
  bundled 0.36.2 and 0.37.0 Hack, Ubuntu, NotoEmoji and emoji-icon fonts finds no U+22EE in either
  release; the 0.36 test drew tofu and the 0.37 kittest harness instead panics on a missing glyph.
- The branch's own lock resolves `egui`/`eframe`/`egui-*` 0.37.0 from crates.io and raises accesskit
  to 0.25.1 and kittest to 0.5.0. The lock churn is large but coherent for a registry-only build.

### 4.2 Findings and required corrections before reuse

1. **Both the PR merge tree and audit merge are unsafe bases.** `b68de8c`
   has a coherent registry lock but a stale tree relative to `4919088`: it lacks
   the entire `crates/platform` crate, drops PDF/media bounds and secure font-fetch
   changes, reverts settings merge behavior, docking permission guards and the
   linear printer-name collision algorithm. `ec8840a` additionally has a stale
   0.36 lock. Never merge/cherry-pick either tree as the migration result.
2. **Reuse only the intended port hunks.** Apply the API changes from `8614e4a`
   and README change from `1764bca` onto recorded current main. Preserve provenance,
   resolve against present source, and regenerate the lock with the final reviewed
   storytold fork/winit set. Include those pins atomically with source adaptation;
   no stock-only intermediate consumer release is planned.
3. **README code still uses `ui.id()`.** `crates/ui/README.md:115` is documentation, not compiled, but
   it is a public example and will fail under `-D warnings` if copied. Change it to
   `ui.scope_id()` in the same documentation pass. The version statement at line 237 must also be
   updated.
4. **The ellipsis change is not a production fix.** The production source still paints `…` and `−`;
   both are present in the bundled Hack/Ubuntu fonts. The old `⋮` was only a test fixture and was never
   covered by the bundled fonts; 0.36 silently painted tofu. Add an explicit glyph-coverage test so
   the next font/default change is caught deliberately, instead of relying on one changed string.
5. **AccessKit role mapping needs contract tests, not just compilation.** The PR changes enum names;
   it does not prove that `Role::Tab` (manually set after `WidgetInfo::selected(Role::Button)`),
   `Role::CheckBox` for stack collapse, `Role::Splitter`, labels, enabled state and selection still
   reach the tree. The 0.37 tree changes from #8581/#8589 make this a real regression surface.
6. **The PR does not test a hidden/minimized `App::logic`.** `craft-ui` has one UI-driven poll path
   (`printing::State::show` calls `poll`) and no logic-only test. Eframe 0.37 calls `App::logic` but no
   `App::ui` while hidden. The API is already public, but the contract is untested.
7. **The PR does not add a publication-compatible check.** Root `[patch.crates-io]` entries will not
   travel to crates.io consumers. The packaged crate must be tested against registry egui 0.37, not
   only against the suite fork.

Decision: reuse only reviewed port hunks from `8614e4a` and `1764bca`. CL-1 uses
an explicit changed-file allow-list: `Cargo.toml`, `Cargo.lock`, `crates/ui/Cargo.toml`,
`crates/ui/README.md`, `crates/ui/src/buttons/mod.rs`, `crates/ui/src/docking/surface.rs`,
`crates/ui/src/docking/tests.rs`, `crates/ui/src/layout/resize.rs`,
`crates/ui/src/layout/tests.rs`, `crates/ui/src/printing/tests.rs`,
`crates/ui/src/tabs/mod.rs`, `crates/ui/src/tabs/tests.rs`,
`crates/ui/tests/docking_workflows.rs` and `crates/ui/tests/visual/fixtures.rs`.
Compare `git diff --name-only` against this list and review the actual allowed-file
hunks: being on the list never permits unrelated reversions. Extra files required
by a proven compatibility fix need a named reason and review before extending it.
In particular, `.github/workflows/**`, `NOTICE`, root `README.md`,
`crates/ui/src/docking/model.rs`, `crates/platform/**`, vendors and non-UI libraries
stay unchanged. Preserve linear `shown_names`, floating-group permission guards,
secure font fetches and all intervening hardening. Run
`moving_a_floating_group_needs_every_member_movable`,
`protected_group_member_blocks_floating_group_resize`,
`collisions_resolve_like_the_pass_by_pass_rule` and `a_long_collision_cascade_stays_fast`.
Root workspace/lock membership must still contain craft-platform.

## 5. Dependency and publication topology

### 5.1 Intended suite graph

For suite validation, `craft-libs` should resolve one and only one copy of each egui-family crate and
one winit:

```text
craft-ui / craft-quit / craft-platform / craft-window
        -> egui 0.37.0 (reviewed published storytold SHA, derived from a4f9a17f input)
        -> egui-winit 0.37.0, eframe 0.37.0, egui-wgpu 0.37.0,
           egui_extras 0.37.0, egui_kittest 0.37.0 (test/consumer)
        -> epaint, epaint_default_fonts, ecolor, emath (same git revision)
        -> winit 0.30.13 (reviewed published storytold SHA including W15P)
```

The root `[patch.crates-io]` must name every used package at the same full SHA. At minimum:
`egui`, `eframe`, `egui-winit`, `egui-wgpu`, `egui_extras`, `egui_kittest`, `epaint`,
`epaint_default_fonts`, `ecolor`, `emath`, and `winit`. Include `egui_glow`
when selected by tests/optional features; metadata determines the complete set. If `egui_system_font_provider` is enabled,
patch it too. Do not patch `egui_plot` or `egui_dock` unless a consumer actually uses them; do not
assume their 0.37 version numbers.

Required checks:

```sh
cargo tree -d
cargo tree -i egui@0.37.0
cargo tree -i winit@0.30.13
```

Acceptance is exactly one egui-family source and exactly one winit source, with no registry
`egui`/`epaint`/`egui-winit` copy beside the git copy. A duplicate is a failure, not a warning.

### 5.2 Publication reachability

`craft-ui`'s production manifest must remain compatible with **stock** egui 0.37.0. The fork-only
behaviors are in dependencies, not in craft-libs source. This is deliberate:

- it keeps `cargo package` usable;
- it lets a consumer choose stock egui 0.37 or the suite fork;
- it prevents a fork-only API (for example `Style::right_to_left` or
  `RawInput::latin_layout_chars`) from leaking into a published shared crate.

Add an isolated `ci/registry-egui` workspace (its own `[workspace]`, not a member of the root
workspace) that depends on `crates/ui` by path without any patch. Run its clippy/tests against
registry egui 0.37.0. Then run `cargo package -p craft-ui` and inspect the generated manifest and
`.crate` contents. A machine-local path patch, an aiongg branch name, or a hidden winit dependency in
the packaged manifest blocks release.

Do not put path overrides into the published manifest. A consumer's `[patch.crates-io]` is the
supported place for the suite fork and winit; document that in the README compatibility section.
The suite's `Cargo.lock` is a lock, not a publication promise.

### 5.3 Versioning

`craft-libs` currently versions every crate `0.1.0` from the workspace. This migration ships as a
new immutable git revision with explicit compatibility notes: egui-facing public types change
identity, so every consumer must move coherently. Do not bump every unrelated workspace crate.
Publishing a crates.io release is a separate release operation requiring a semver/version plan;
it is not authorized or required by the git-based suite migration.

### 5.4 Vendors and nested workspaces

The only vendored packages are PDF readers/writer, not egui:

- `vendor/krilla` (0.8.2), `vendor/hayro` (0.7.1), `vendor/hayro-interpret` (0.7.0),
  `vendor/hayro-syntax` (0.7.2), `vendor/hayro-jbig2` (0.3.0), `vendor/hayro-cmap` (0.1.0)
  and `vendor/hayro-postscript` (0.1.0).
- Reader provenance and patch ledger: `vendor/README.craft-reader.md`. The first reader transfer
  was from PdfCraft `7437bde378f43856eada40af25b983812e615e8a`; the ledger records subsequent
  transfers and fixes. Writer provenance and patches are recorded separately in
  `vendor/krilla/README.craft.md`. Each source manifest records the relevant source hashes.
- Equivalence decision: keep the 0.7 backend and its documented compatibility patches. They are not
  replaced by the egui migration. When reconciling current main, preserve its reader/writer fixes
  and provenance byte-for-byte; do not independently re-vendor or remove them.
- Removal/test condition: only a separate PDF backend migration may change them; their existing
  `cargo test -p craft-pdf --all-features` and `scripts/test-pdf-backends.sh` evidence remain the
  gate. Run these checks again after a main reconciliation changes the vendors; default-feature
  workspace tests alone do not qualify the PDF backend. The root `[patch.crates-io]` must keep
  these entries while `craft-pdf` uses them.

`crates/pdf/read-only-contract` is a nested workspace with its own `Cargo.lock` and a single
`hayro-syntax` patch. It does not depend on egui and is unaffected by the workspace egui lock; do not
edit its lock as part of this migration. There are no vendored egui, eframe, egui-winit or winit
sources in this repository.

## 6. Root-`Ui` and lifecycle design

### 6.1 Boundaries

craft-libs must not grow a blanket `Context` compatibility layer. The correct boundaries are:

- **Application lifecycle:** eframe owns `App::logic` and `App::ui`. `logic` runs before each
  UI pass, possibly repeatedly in one frame, and in hidden mode when a repaint
  is requested; `ui` may not run at all while hidden. Code that must
  drain queues, service print jobs, autosave, or decide a close must live in `logic`.
- **Root UI:** `App::ui` receives the root `Ui`. `craft_ui::docking::DockArea` and every other component
  take `&mut Ui`. They must never call `Context::root_ui` from inside their own rendering; that would
  panic when the root UI is already borrowed (`Context::root_ui` docs at 0.37.0).
- **Painted output:** `DockArea::show` returns `DockOutput` actions and does not mutate the layout.
  The app applies them after rendering. This remains the command/undo/persistence boundary.
- **Hidden/minimized:** no component may rely on a paint to make progress. `printing::State::poll`
  already takes `&Context` and can run in `logic`; `show` is a fallback. Background receivers,
  autosave timers and playback must be app-owned and continue in `logic`. Pending
  work must arrange a wakeup with `request_repaint`/`request_repaint_after`; hidden
  polling is not a continuous scheduler. Test repaint-driven delivery explicitly.
- **Multiple passes:** `Context::run_ui` can call its closure more than once after `request_discard`.
  Both hosts call logic/UI inside that closure. Queues/actions must be consumed
  once and deadlines use monotonic time. Count observable effects across all passes;
  do not hide duplicates by counting only the final pass or altering the test host.
  After a completed run_ui the pass index is zero, but events/time remain stale in
  run_logic. Fresh input belongs to the known UI phase, including forced hidden paint.

### 6.2 Preserve lazy printing and hidden result delivery

Current behavior:

- `printing::State::show` calls `refresh` on first display and `poll` every displayed frame
  (`crates/ui/src/printing.rs:205-209`).
- `poll` drains discovery/submission receivers and calls `request_repaint_after(100 ms)` while work is
  pending (`printing.rs:162-201`).
- WordCraft and GridCraft already call `poll`/`poll_printing` from `logic`, and GridCraft also calls it
  from `ui`; repeated polling is currently harmless because `try_recv` consumes once. A window
  that has never opened print should not start printer discovery merely because `logic` runs.

Keep `refresh` as the explicit discovery trigger and `poll` as the idempotent receiver drain.
Retain `show`'s lazy first-display behavior. Apps may trigger discovery from the print-open action
before the page is drawn, then poll pending work in `logic` while hidden. Do not add an eager
`tick` API or new startup process/IPC work as part of this migration. Change production code only
if a regression demonstrates a missing transition. `Settings`, `Feedback`, `begin`, `ready`,
selection and job ownership stay unchanged.

Contract tests:

1. `print_poll_is_lazy_without_open`: run repeated `poll` calls through `Context::run_logic` before
   any print action; assert no discovery worker starts and no periodic wakeup is requested.
2. `print_result_arrives_without_ui`: trigger `refresh` once, send a fake printer list and run
   `poll` through `run_logic`; assert printers/feedback update without a `Ui`. Subsequent `show`
   and discarded UI passes must not start another worker or consume a result twice.
3. `completed_work_requests_no_more_repaints`: after a submission result is delivered, run 60
   `run_logic` passes and assert no `request_repaint_after`/immediate repaint is requested by
   `State`.
4. `queue_default_and_selection_survive_logic_only_poll`: assert a still-present queue remains
   selected and a removed queue cannot be submitted.

### 6.3 Autosave, undo and side effects

No autosave, playback or undo engine lives in craft-libs. The migration contract for consumers is
therefore explicit:

- queue drains, autosave, audio/playback polling and save-file completion callbacks go in
  `App::logic`, not `App::ui`;
- painting only presents state; it must not be the sole trigger for a save/undo transaction;
- `DockArea` action output is applied once, after rendering, through the app command path;
- any `Context::request_discard` path must be idempotent; tests use a call counter to prove exactly
  one committed action or one undo step.

These are app acceptance criteria, not claims that this repository's tests can prove application
behavior.

## 7. Component-by-component migration

### 7.0 Shared-helper policy

The 0.37 API renames are direct calls, not a reason to build a compatibility layer. Apply this rule:

- **No wrappers** for `Id::unique`, `Role`, `Ui::scope_id`, `ScreenshotCallback`,
  `Context::set_language`, `Response::accessible_name` or `Context::run_logic`. Applications call the
  egui API; craft-ui exposes only component-specific semantics.
- **Justified mechanism:** a bounded-lifetime capture completion slot, reviewed with two concrete
  existing craft-ui consumers; section 7.8 keeps request IDs and application policy outside it.
- **Existing print API:** retain lazy `refresh` and idempotent `poll`. No eager discovery helper.
- **Docking direction:** retain the persisted first/second split and tab model. Use an explicit
  opt-in DockArea direction policy from section 7.7, with matching hit coordinates;
  no hidden global mirroring mode.
- **Not shared:** fonts, themes, localization catalogs, autosave, undo history and native pickers.
  They remain application-owned, as the existing component audit requires.

### 7.1 IDs and scopes

Anchors: `crates/ui/src/tabs/mod.rs:117` (`Tab::new`), `:141` (`ui.interact`),
`crates/ui/src/docking/surface.rs:539` (body child), `:600` (header child), `:761` (floating `Area`),
`crates/ui/src/layout/resize.rs:82` (handle), `crates/ui/src/printing.rs:210` (`push_id`).

Rules:

- Caller-supplied widget IDs stay caller-owned and global. Use `Id::unique(("app", panel))` or an
  app-owned `IdSalt`; never derive an ID from translated text.
- `Tab::new` and `ResizeHandle::new` keep taking `Id`. The public signatures do not need a new wrapper.
- `Ui::new_child(UiBuilder::new().scope_id(...))` must use a scope id that is globally unique. The
  body scope is `self.id.with(("body", panel))`; the header scope is
  `self.id.with(("header", selected))`; they are distinct. Do not replace them with `id_salt` unless
  the child is intended to move with its parent.
- `printing::State::show` continues to use `ui.push_id(id, ...)`; the caller's `id` remains the
  stable identity of the print block.
- `Id::unique` is the same hash as the old `Id::new`, so test behavior is intended to be identical.
  Add a test that the same source yields the same `Id` across frames and that two calls with
  different panel payloads differ.

### 7.2 Accessibility roles and names

Anchors: `buttons/mod.rs:84`, `tabs/mod.rs:143-148`, `docking/surface.rs:419-421,569,628,769`,
`layout/resize.rs:157-173`, `printing.rs:213-254`.

Required mappings and assertions:

| Control | Role / metadata | Assertion |
| --- | --- | --- |
| Icon button | `Role::Button`, `selected`, label, enabled | AccessKit role Button; `Toggled::True/False`; exact label; disabled flag |
| Tab | widget info starts Button, then node role `Tab`; `clear_toggled`; selected | Role Tab, selected, no toggled flag, label exact; duplicate labels have distinct IDs |
| Stack collapse | `Role::CheckBox`, selected = open | Role CheckBox, toggled/selected matches open, label exact |
| Resize handle | `Role::Splitter`, orientation, numeric value/range/step, Increment/Decrement/SetValue | orientation perpendicular to movement; actions; SetValue delta; NaN ignored |
| Floating group grip | `Role::Button`, move label | Role Button, label; not focusable as a splitter |
| Overflow | `Role::Button`, panels label | Role Button, label, menu opens and entries keep logical labels |
| Tab menu actions | `Role::Button`, logical labels | custom visual text does not replace AccessKit label |
| Print printer ComboBox | labelled by visible printer label | `labelled_by` present; `get_by_label("Printer")` finds it |
| Print copies DragValue | labelled by visible copies label | label present; `check_accessibility` passes |
| Print grayscale checkbox | own label | Role CheckBox, label present |

Do not disable `Harness::check_accessibility` globally. If a fixture deliberately uses an unnamed
input, scope the exception to that fixture and document why. `egui_kittest` 0.37 fails unnamed input
widgets by default (PR #8589); that is a feature, not a migration nuisance.

### 7.3 Panels, docking, order, clip and floating layers

`DockArea` does not use `egui::Panel`; it renders a snapshot into the caller's `Ui` and uses
`Area::new(id).order(Order::Middle)` for floating groups
(`crates/ui/src/docking/surface.rs:761`). The app's panel structure remains outside.

Required integration tests:

1. **Panel order and clip:** in a root `Ui`, add `Panel::left("left").exact_size(200).show(...)`, then
   `CentralPanel::default().show(ui, |ui| DockArea::show(...))`; assert the dock gets the remaining
   width, the left panel is not overdrawn, and body `Ui` clip is a subset of the parent clip.
2. **Nested panels:** a top panel containing a horizontal `Panel` and a central panel; assert order
   and no double allocation. This checks 0.37's `Panel`/`CentralPanel` last rule.
3. **Floating layer order:** two floating groups, `raise_panel` on the second, assert the top layer
   and tab focus; assert the serialized `Layout` is unchanged by `raise_panel`.
4. **IDs after move:** move a panel into a floating group and back; assert the body `scope_id` is the
   same before/after and that a focused text field retains its edit buffer.
5. **No document allocation in floating-only mode:** existing test
   `floating_only_surface_does_not_allocate_the_callers_document_region` remains mandatory.
6. **Clip and popup:** open the overflow `Popup` and tab context menu near the viewport edge; assert
   popup rect stays inside the screen, menu labels are logical, and the parent clip does not clip the
   popup away. 0.37's popup tree changes (#8581) make this a real check.

Do not turn `DockArea` into a native multi-window manager. Native viewport creation, reparenting and
cross-monitor coordinates remain host-owned. `Action::Float` means an in-viewport floating group.

### 7.4 Tabs, split geometry, resize handles, toolbar and theme

- `tabs::fit` is direction-agnostic and pure. Keep its exact width/overflow assertions. Add a
  64-tab and a 4096-tab bounded-work test; the latter must still be linear and must not allocate
  unbounded storage.
- `layout::split_rect` and `SplitLayout::resized` stay pure and f32/f64 bounded. Add property tests
  with extreme finite values and non-finite rejection; do not accept a `None` as a silent panel
  disappearance in app adapters.
- `ResizeHandle` focus lock, arrow keys, AccessKit actions and pointer drag deltas stay. Add a test
  that a focused handle consumes only its axis arrows and leaves Tab traversal intact.
- `fit_toolbar` stays ordered-prefix only; add a test that every input index is either shown or in
  overflow, and that the overflow control is reachable when the width is exactly the overflow width.
- `WidgetPalette` keeps preserving borders/expansion. Add a test that a 0.37 default-style change
  does not overwrite an explicit app border or expansion.
- `DockStyle::from_ui` should document that 0.37's default radius/spacing values are inherited; apps
  with exact designs must pass explicit values.

### 7.5 Fonts, glyphs and custom catalogs

Anchors: `docking/surface.rs:425` (`−`/`+`), `:629` (`…`), `printing.rs` labels,
`crates/ui/tests/visual_regression.rs` (font-free fixtures and SVG raster checks).

- `craft-ui` production still disables egui default features. This is correct: the app chooses fonts.
- The `crates/ui` dev-dependency keeps `default_fonts` so component tests can shape labels; keep it.
- 0.37's `default_fonts` no longer includes monochrome emoji fonts, and the 0.37 kittest harness
  panics when nothing can draw a glyph. That is why the `⋮` fixture had to change even though 0.36
  had silently drawn tofu. `…` and `−` are present in Hack/Ubuntu; verify with:
  `ctx.fonts_mut(|f| f.has_glyphs(&FontId::proportional(12.0), "…−+"))` after one `run_ui` pass.
- Add a test using `MissingGlyphPolicy::Panic` (or `Harness` default) with `⋮` as a deliberate
  negative fixture: it must fail. Then use `…` in the actual component and assert `has_glyphs`.
- Do not call `allow_missing_glyphs()` on a whole suite just to make a test green. If arbitrary
  script fixtures are intentional, scope the policy to that fixture.
- SVG catalogs remain trusted compiled-in bytes and are not a font fallback. `SvgIconSet` still
  requires a unique URI prefix per catalog/style; egui caches by URI. Test SVG at 12/24/48 logical
  pixels and exact RGBA.

### 7.6 Quit, platform and window helpers

- `craft-quit::on_close_request` already expects `egui::Context`, `ViewportCommand::CancelClose` and
  the root viewport. It compiles against 0.37. Add a test with `App::logic`-style ordering: app
  cancels close first, then `on_close_request` sees the cancellation and does not quit.
- `craft-platform::DeferredClose::update` scans `ViewportCommand::Close`/`CancelClose`; keep its test
  that a pending picker holds close and releases exactly once. The 0.37 `ViewportCommand::Screenshot`
  callback does not change this, but apps must not treat a pending screenshot as a pending close.
- `craft-platform::sync_native_window_theme` is an `egui::Plugin::output_hook`. In 0.37 the plugin
  API still calls `output_hook` after the pass; keep the test that a manual `SetTheme` is not
  overwritten and that child viewport outputs receive the hint.
- `craft-window::decorate` only touches `ViewportBuilder`; no 0.37 API change. Keep the Linux/BSD
  Wayland probe and the `WAYLAND_SOCKET` protection. Do not call it more than once at startup.
- `craft-quit` menu code uses `muda`; 0.37 does not change muda. Do not add a second native menu
  abstraction.

### 7.7 Input, IME, RTL, clipboard and PRIMARY

craft-libs itself does not implement text input, IME, clipboards or RTL shaping; it consumes egui
widgets and labels. The fork changes that matter:

- **E5/E12/E13/E15/E16/E20:** bidi shaping, visual order, grapheme editing and language-aware layout
  affect every `Label`/`TextEdit` inside a `DockArea` body. Test Arabic/Hebrew mixed labels through
  the fork fixture, not stock 0.37. Keep ASCII geometry tests unchanged as the no-regression baseline.
- **E9 and docking direction:** built-in egui chrome follows the fork's direction,
  but custom docking uses explicit geometry. Add an opt-in `DockArea::direction`
  builder with a stock-compatible `DockDirection::{LeftToRight, RightToLeft}`;
  default remains LTR. Applications pass their UI direction; craft-ui does not
  read fork-only style fields or infer language from labels.
  - Preserve serialized split first/second, axes, ratios, floating rectangles and
    logical tab order. Changing UI direction must not rearrange a saved workspace.
  - Mirror horizontal tab-strip visual order and leading/trailing controls. Painting,
    clipping, hit tests, accessibility order and drag insertion use the same computed
    rectangles. Convert visual insertion index back to logical model index once.
  - Split-pane geometry and physical splitter drag directions stay unchanged.
    Floating resize grips use bottom-trailing placement (right in LTR, left in RTL);
    RTL resize keeps the right/top edges fixed: for pointer delta `(dx, dy)`,
    clamp `(width - dx, height + dy)` to the same limits, then emit
    `Action::MoveFloating` with `[old_right - new_width, old_top, new_width, new_height]`.
    LTR keeps left/top fixed and clamps `(width + dx, height + dy)`. Apply the existing
    screen clamp afterwards. Horizontal splitter motion to the right increases the
    physical left pane and decreases the right pane in both directions; vertical
    motion downward increases the top pane. Assert these rectangles/ratios and
    keyboard-generated deltas under both policies and at min/max/screen limits. For a focused
    floating resize grip, ArrowLeft supplies negative physical dx and ArrowRight positive dx:
    RTL ArrowLeft increases width and moves the left edge left while keeping right fixed;
    RTL ArrowRight decreases width and moves left right. LTR uses the opposite width
    effect with left fixed. ArrowDown grows height; ArrowUp shrinks it in both directions.
  - Physical left/right keyboard traversal follows visual tab order; panel IDs and
    focus stay stable across direction changes. Vertical behavior is unchanged.
  - CL-3 tests LTR/RTL click/drag/reorder/resize, keyboard focus, protected floating
    groups and unchanged serialized workspace. Deliberately mirror paint without
    hit coordinates to prove failure detection. This is a visual policy, not a
    schema migration or wholesale locale redesign.
- **E17/E18:** `Context::set_language(Some("ar".to_owned()))` sets the AccessKit root language; `UiBuilder::language`
  marks content language; eframe's `Frame::set_ui_language` is the convenient host hook. Craft-ui
  should not cache a language; pass localized text and let the app set language once per change.
- **E19:** Latin layout characters affect app shortcuts, not craft-ui geometry. App tests must cover
  Ctrl+Q on Russian/Arabic/Hebrew and Dvorak Command layouts; do not infer shortcut behavior from
  `Key` alone.
- **E21:** TextEdit deletes the exact UTF-8 bytes an IME replaces and groups the deletion with the
  commit. App undo tests must assert one undo step for a press-and-hold/dictation replacement.
- **E21b/P7b (new prerequisite):** the candidate does not send outgoing surrounding text to W27.
  The reviewed fork must add the bounded transient IME payload, built-in TextEdit producer and
  Wayland adapter before CL-1's fork contract can pass. Check deletion-only and delete/commit
  batches, selection/focus changes, password clearing, two windows, UTF-8 boundaries, preedit
  exclusion and the 4,000-byte window. Stock-compatible craft-ui production code uses ordinary
  TextEdit and needs no fork-only payload API; the fork contract fixture exercises it explicitly.
  The eight custom-editor consumers need their own adapters as specified in the suite rollout.
  CL-1 requires the final reviewed E21b/W27 revisions with an explicit input-owner/context
  generation present for both `Available` (including an empty document) and `Unavailable`, and clear/invalidate
  operations. Clear both caches and protocol state through a committed disable/enable boundary
  when the owner changes or text becomes unavailable; ordinary updates preserve composition.
  Cancel old pending preedit/commit/delete state on leave/reset/disable. Within the current focus
  epoch, text-input-v3 `done` with a mismatched serial still applies incoming edits once but defers
  outgoing state until a matching serial; old-epoch events cannot edit the new owner. The latest
  completed UI pass is authoritative, including `IMEOutput = None`; do not retain the earlier
  payload through `ime.or(self.ime)`. Test password/unavailable/over-limit transitions, queued
  deletion then refocus, reset, two windows and text-to-non-text discarded-pass output.
  Final SHAs are execution prerequisites. `8b35388b` remains a comparison input, not an interim
  production pin: CL-1 waits for the completed reviewed/published dependency series.
- **E4 + upstream #8428:** the existing candidate's arboard PRIMARY fallback is
  replaced by the selected new winit W15P path on the same connection/event queue,
  using SCTK's per-seat PRIMARY objects. This must land before egui P2b and CL-1.
  Test pointer- and keyboard-triggered ownership, including keyboard-only focus, with
  fresh triggering serials scoped to their seat/origin (no arbitrary first-seat fallback),
  owner/seat changes, empty/absent offers,
  self-paste and canceled/slow transfers on wlroots/KWin/Mutter. Keep password and
  unchanged-selection protections; no second regular data device. An isolated
  PRIMARY-only fallback needs measured necessity and review; ordinary paste is
  not PRIMARY acceptance. W15 and W15P must use bounded asynchronous transfer handles,
  a total monotonic deadline, byte/count limits and bounded workers. The existing two-second
  per-`WouldBlock` timeout is not a total bound. Egui consumes completion only for the still-live
  paste request/owner/input generation; cancellation and late completion cannot paste elsewhere.
  Verify responsiveness during slow trickles and oversized/concurrent offers.

### 7.8 Screenshot callback correlation (downstream contract)

craft-libs has no screenshot code; multiple suite applications do. The migration must replace
`ViewportCommand::Screenshot(UserData)` plus `Event::Screenshot` matching with one-shot callbacks:

```rust
// Proposed ownership, not an existing API:
// app request record -> one result slot -> optional image + owning-viewport wakeup
// screenshot callback -> weak reference to that slot
// app logic -> take the image once, validate request, encode/write/reply
```

Requirements:

- `ScreenshotCallback::new` / `Context::request_screenshot` is one-shot. Clones share one callback;
  complete it at most once.
- The request record owns one completion slot and the callback holds a weak reference
  to that exact slot. Correlation comes from ownership, not callback order or a second
  global token map. Drop/cancel releases the slot and rejects late results.
- If the window is hidden, send `ViewportCommand::RequestPaintWhileHidden` or the screenshot command
  itself (a pending screenshot already requests a paint). A hidden `App::ui` is not run otherwise.
- In a custom/headless renderer (for example LightCraft's headless path), retain the
  `ScreenshotCallback` and call `complete(image)` with the next rendered image; do not reconstruct a
  fake `Event`.
- Test two outstanding requests with distinct tags, callback-once behavior, no callback without a
  request, and a hidden-window request. Test E2 surface fitting: the returned image may be smaller
  than the window when surface fitting reduced it. The integration render_scale is
  private: derive x/y image scale from returned image dimensions and the recorded
  logical viewport rectangle for that capture, including pixels-per-point metadata.
  Verify expected aspect/rounding. A resize/DPI generation mismatch retries within
  the deadline or returns an error; never combine old image pixels with current geometry.

Add a narrow completion-slot helper for existing craft-ui consumers after reviewing two concrete
adapters together (DesignCraft and GridCraft are the selected first consumers). Their tokens, paths, settle
delays, deadlines and replies remain app-owned. The callback holds only a weak slot reference;
avoid a strong Context/callback cycle, release locks before wakeup, and drop late results after
cancellation. The app enforces aggregate request/image limits. A bounded nonblocking channel is
the measured fallback if slot stress tests expose a problem. Apps without craft-ui need not add
it solely for capture. See the suite rollout plan for the selected ownership contract.

Admission accounts for in-flight captures as well as completed slots: reserve a checked byte
budget before issuing a capture, using the allowed surface dimensions, and cap request count
independently. Defaults: 4 outstanding requests, 128 MiB/image, 256 MiB aggregate reserved
in-flight/completed image bytes; stricter app budgets are allowed. Record known issuance UI
logical bounds and a geometry epoch. Reject/retry any resize/DPI/zoom/surface epoch change
before save, even a change back; the callback contains pixels only, not frame metadata.
Revalidate dimensions before issuance after resize/DPI changes. On completion,
reject an oversized or stale image and release both its memory and reservation; account for the
renderer/readback's transient allocation separately in peak RSS. A post-callback size check alone
does not prove that peak capture memory was bounded. Stress concurrent 4K captures, resize races,
timeouts and canceled records; retained image bytes must return to zero after drain/shutdown.

## 8. Fork-specific patch plan: E1-E21

The fork's `PATCHES.md` is the source for the table below. The upstream column reflects the inspected
tag `0.37.0` and open PRs, not a claim about `main` after 2026-10-15. Removal is always conditional on
a released egui version in use containing equivalent behavior and on the full suite tests passing
without the patch.

| Patch | Commit(s) | Behavior | Upstream status at 0.37.0 | craft-libs relevance and required test |
| --- | --- | --- | --- | --- |
| E1 | `8f3abf06` | eframe `NativeOptions::viewport_hooks`: every viewport's native window on create/destroy and every viewport's `RawInput` before egui, including immediate viewports | not proposed | No direct craft-ui use. Compile-only fork contract; native window identity is app/craft-tablet. Test hook installed/replaced and one call per viewport in egui repo. |
| E2+E3 | `9dd799ab` | egui-wgpu fits a surface to `max_texture_dimension_2d` and returns `ConfigureSurface`/`CreateRenderer` errors instead of panicking | not proposed; addresses open #8361 | Required for large/multi-monitor windows. Test `surface_fit` edge cases and an oversized surface; report errors rather than abort. Screenshot pixel geometry must account for reduced surface. |
| E4 | `66bacf23` | egui-winit uses winit's Wayland data device for regular clipboard, falling back to arboard; smithay not started | emilk/egui#8657 draft; winit#4729 open | Required on Hyprland/KWin; needs W15 plus new W15P for PRIMARY. Test regular/image/drop and PRIMARY separately, serial/seat/ownership/cancel cases, and no second regular data device. |
| E5 | `0e27d87e`, `777977b7`, `f7e89e23`, `5b2b8e19` | Bidi shaping, visual row order, marks/end caret/selection, no-default-fonts tests | emilk/egui#8577 open, plus #8698 review fixes | Affects all app text and DockArea labels. Test Arabic/Hebrew mixed rows, selection across boundaries, marks, end caret, ASCII unchanged; run with default_fonts off and on. |
| E6 | `0abfabcf` | `request_focus` on the already-focused widget does not interrupt IME composition | not proposed | App TextEdit behavior. Test focus to same widget preserves composition; focus to another widget commits/cancels as intended. |
| E7 | `0670c58a` | Windows letter keys from virtual-key code on layouts without A-Z | not proposed | App shortcuts. Test Russian/Arabic/Hebrew layout key names and no duplicate text. |
| E8 | `1b03c10f` | TextEdit does not clip glyphs that overshoot the row | not proposed | App text clipping. Test Thai/Vietnamese/Arabic marks in single-line and wrapped fields. |
| E9 | `b2f5cafc`, `e1341a3d`, `5105330d`, `75a5b3d3` | `Style::right_to_left` mirrors menus, scroll bars, headers, title bars, combo boxes, checkboxes, popups, resize grips | not proposed | Required for RTL UI. Test built-in chrome in both directions; `DockArea` manual tabs/splits remain explicit. Do not let this alter persisted layout semantics. |
| E10 | `dc90c13e`, `c8e573f7`, `34b2c0d4` | eframe web: named keys not typed as text; letter keys from `KeyboardEvent.code`; `lang` on hidden input and speech | not proposed | Web apps. Test browser key layout, IME, screen-reader language; wasm check is compile-only. |
| E11 | `92e61afd` | epaint rasterizes each glyph on a single-threaded vello context, avoiding vello multithreading panic | emilk/egui#8641 open draft | Required when any app enables `vello_cpu/multithreading` (VectorCraft). Test rasterization with the feature on and no thread-per-glyph panic. |
| E12 | `e955bb18`, `485c0b89`, `0185887c`, `7015ac23`, `4067b2c4`, `620e070e` | `LayoutJob::direction`, RTL paragraph alignment, visual-end ellipsis, UAX #9 L1, linear reorder, bidi controls keep levels | not proposed; follows #8577 | Test forced LTR/RTL, neutral paragraphs, wrapped RTL, elision, long mixed paragraph linearity. Affects tab label widths. |
| E13 | `5e4da7f4` | Cursor movement/hit-test/Delete/Backspace by grapheme cluster | not proposed | App text editing. Test combining marks, emoji ZWJ/flags/keycaps, Devanagari, Thai, Hangul, CRLF; hit test every x in LTR/RTL. |
| E14 | `cbd77549`, `b3722ac0` | Fallback fonts share baseline and grow the row; `FontTweak::fallback_align` default unchanged | not proposed | CJK/Indic fallback. Test row height, DockArea `tab_height` clipping, and default center behavior unchanged. |
| E15 | `f1cae609` | Pluggable `LineBreaker`, grapheme-safe emergency breaks, `LayoutJob::language` | not proposed | App line breaking. Test `SimpleLineBreaker` old rules, custom breaker receives language, hard breaks and emergency cluster breaks. |
| E16 | `b3ef9771`, `bed6dfe3` | Visual cursor movement with affinity, Home/End reading order, per-cluster password mask, neutral paragraph inherits direction | not proposed | App text editing. Test bidi arrow behavior, Home/End, selection, password masking; no craft-ui API change. |
| E17 | `520435df` | `Context::set_language`, `UiBuilder::language`, `Response::accessible_language`; AccessKit root language; one text run per bidi level | not proposed | Craft-ui labels/accessibility. Test root language, per-widget language, mixed bidi runs, screen-reader names. |
| E18 | `89de8f3c` | eframe `Frame::set_ui_language`; web speech/input language fallback | fork-only | App host hook. Test native and web language propagation; no craft-ui cache. |
| E19 | `d64047bf`, `642b74f7`, `baeb4515` | One Latin-letter shortcut rule on Windows/macOS/Linux; `RawInput::keyboard_layout_is_latin`; per-key Latin layout chars | fork-only; needs W25 | App shortcuts. Test non-Latin layouts and Dvorak-Command; `Key` alone is insufficient. |
| E20 | `23955535` | Widget text laid out in the language of its `Ui` | fork-only (with E15/E17) | Tab/menu/label shaping language. Test Arabic/Hebrew labels inside DockArea and menu; ASCII unchanged. |
| E21 | `a70470b0` | `ImeEvent::DeleteSurroundingBytes`; TextEdit deletes exact bytes in the commit's undo step | not proposed; needs W26/W27 | App IME. Test exact-byte deletion, one undo step, combining accent/flag/ZWJ replacement, and no over-deletion. |
| E21b/P7b | New, no commit yet | Bounded outgoing surrounding-text payload, TextEdit producer and egui-winit W27 adapter | Confirmed missing from candidate | Fork-only contract tests cover logical text, UTF-8/preedit mapping, 4,000-byte limits and focus/password clearing; custom app editors supply their own adapters. |

Decision: keep E1-E21 and implement E21b before the target is frozen; remove none during this migration. The removal
condition for every row is a released egui version in use with equivalent behavior, the fork test
suite passing without the patch, and the app workflows passing. E5 is the dependency root for E12,
E13, E16, E17 (language runs), E18 and E20; E19 and E21 are independent but require specific winit
patches. A removal decision for one patch must consider its
dependents, its tests and the winit side. The fork's own tests are numerous: epaint text layout tests
include bidi order, mixed Hebrew/Latin, cursor hit-testing, selection boundaries, grapheme clusters,
fallback baseline, line breaking and RTL wrap; egui integration tests include `ime_focus`,
`text_edit_overshoot`, `layout_language` and `ime_delete_surrounding`. Those tests are the first
acceptance layer, not the only one.

## 9. Test plan and regression assertions

### 9.1 Commands and evidence

All commands run through `maintenance/task.py run/status/finish`, never directly, with a fresh task,
exact source, durable evidence and a unique job ID. Use the allocated `CARGO_TARGET_DIR`; do not use a
shared target.

Minimum gates, in order:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo clippy -p craft-ui --all-targets --all-features --locked -- -D warnings
cargo test -p craft-ui --all-features --locked
cargo check -p craft-ui --all-features --locked --target wasm32-unknown-unknown
```

Then the fork/registry contract and package checks described in section 5. Record actual executed
test counts, exit codes, `rustc -Vv`, `cargo -V`, lock hashes, feature list and the full command line
for each run. Do not call a historical audit log a fresh run.

### 9.2 Stable fixtures and exact assertions

Use fixed `RawInput.screen_rect`, fixed `time`, `pixels_per_point` 1.0/1.5/2.0, both themes, and
`animation_time = 0.0`. No test may update a golden automatically.

Required assertions:

- **Pixel/mesh:** 372 positive visual pairs exact RGBA and exact vertices/indices/texture IDs/clips;
  36 deliberate color/spacing/stroke changes must differ in both. If a 0.37 default changes a fixture,
  diagnose the component or deliberately change the independent reference; never accept a new golden
  to hide a visual regression.
- **Geometry:** exact `TabFit` widths/overflow for the existing fixtures; split first/divider/second
  rectangles exact; 64-tab and 4096-tab work bounded; `fit_toolbar` every index reachable.
- **IDs/focus:** tab/body IDs equal across frames and moves; duplicate labels distinct; focus survives
  a panel move; `scope_id` children do not collide.
- **Actions:** `DockArea` returns the expected single `Move`/`Float`/`Close`/`Resize` sequence; the
  layout snapshot is unchanged until the caller applies it; 800 deterministic moves never lose or
  duplicate a panel.
- **Persistence:** `Layout` and `Action` serialize to a fixed JSON fixture and back without changing
  panel identity, active index, fixed sizes, accordion state or floating rect. Unknown fields are a
  deliberate schema decision, not an accident.
- **Accessibility:** exact roles/names/selected/enabled as in section 7.2; `Harness::check_accessibility`
  passes; unnamed input fixture (deliberate) fails when the name is removed.
- **Lifecycle:** no print worker starts before print is opened; once requested, results are
  delivered via `run_logic` without `run_ui`; completed work requests no further repaint and
  repeated logic/UI calls do not start a second worker.
- **Glyphs:** component-painted glyph set (`…`, `−`, `+`) passes `has_glyphs` with bundled test fonts;
  missing `⋮` under panic policy demonstrates the test detects a font regression.
- **RTL/fork:** in the fork contract workspace only, Arabic/Hebrew mixed labels keep logical order,
  tab widths and hit rects remain finite, built-in menus mirror, and an ASCII fixture is byte-identical
  to the stock 0.37 result (where the API is shared).
- **Screenshot:** two tokens, callback once, hidden-window request, surface-fit scaling; app fixtures
  live in app repositories, not craft-libs.
- **Panic policy:** no new `unwrap`, `expect`, `panic`, `todo`, `unimplemented` or `unreachable` in
  production code; crate `lib.rs` deny headers stay.

### 9.3 Deliberate failure probes

The existing 36 visual mutation comparisons are the primary pixel/mesh probes. Add these targeted
negative checks:

1. Render an icon button with the label omitted or role changed to `Label`; the accessibility test
   must fail.
2. Remove `labelled_by` from the print `DragValue`; `Harness::check_accessibility` or the label query
   must fail.
3. Render `⋮` under `MissingGlyphPolicy::Panic`; the fixture must fail.
4. Feed the same body `scope_id` to two simultaneously rendered panels; a duplicate-ID test must fail.
5. Change a tab-strip width by one point or reverse the overflow order; exact geometry must fail.
6. Call the print discover closure twice; the one-worker counter test must fail.
7. Deliberately misroute a capture slot to another request; correlation must fail. Completing
   clones twice is a positive one-shot test and must still yield exactly one result.

Document the expected failure for each probe and keep it out of release acceptance.

## 10. Performance protocol and thresholds

### 10.1 Baseline capture and the implemented CPU harness

The harness is test-only work. Prepare separate baseline/control and candidate qualification
worktrees with the same measured work, cases and instrumentation; record their exact commits,
complete patches, harness/lock hashes and any reviewed API compatibility adapter. The original
plan proposed `49190884278f16c9a9f5823d7698a8ffb5deb062` as a baseline; that ref alone does not
qualify a comparison. Preserve the published source and existing control worktrees. Run correctness
gates before measuring, then alternate paired baseline/candidate runs under the same conditions.

Environment to record exactly:

- host CPU: Intel Xeon E-2176G, 12 threads (6 cores), 3.70 GHz base, 125 GiB RAM (the observed host);
- current host inspection finds i915 PCI `8086:3E96` and an accessible render node; no display
  session is inherited. Qualify the actual selected adapter/driver/backend with a managed job.
  Historical llvmpipe results are software-renderer evidence only, not current hardware discovery;
- Windows row (if used): the Windows VM's Microsoft Basic Render Driver (CPU), not a physical GPU;
- no real-GPU measurement has been performed for this task. Hardware availability alone does not
  establish renderer, compositor or presentation acceptance.

The implemented target is `crates/ui/examples/ui_hotpaths.rs`, a headless JSON-emitting example,
not a Criterion bench target. It measures:

- pure `tabs_fit`, `split_rect`/`resized` and `fit_toolbar` cases across the encoded sizes/scales;
- dock layouts with four split/floating panels or a 32-panel tree at 800×600 and 1440×900,
  at 1× and 2×;
- left-panel plus central-dock integration, with actual rendered bodies, geometry and interaction
  checks. Frame timing includes `Context::run_ui` and `Context::tessellate` together.

Printing receiver/poll lifecycle, separately timed text tessellation, fresh-process font discovery,
GPU pipeline construction and native child/input behavior need their own evidence. They are not
implemented measurements of this example. Texture deltas are counted, never uploaded; no window,
GPU renderer, compositor, physical input or allocation-counting allocator runs here.

Named cross-repository acceptance owners: egui fork section 10 owns fresh-process
DX12 renderer/pipeline construction (#8736), and SoundCraft/PdfCraft Windows rows
exercise their real options. Its selection-then-toolbar test owns #8763/#8764;
craft-ui adds select-text-then-dock/button-action integration coverage. Its hidden
parent/deferred-child fixture owns #8748; DesignCraft's native child-window scenario
provides the consumer evidence. Link actual results to CL-4, rather than claiming
craft-libs' library harness independently validates native DX12 or child windows.

Build and run the implemented example through the owning `task.py` resource, keeping its allocated
target/source/evidence paths. Build separately from measurement so Cargo startup/compilation is not
mistaken for executable RSS or wall time. For example, inside the parent's managed jobs:

```sh
cargo build --release -p craft-ui --example ui_hotpaths --locked

# Use a distinct durable output for each paired attempt; repeat in the candidate's own resource.
# CARGO_TARGET_DIR is supplied by task.py, never overridden or shared across sources.
CRAFT_UI_PERF_LABEL=baseline-attempt01 \
CRAFT_UI_PERF_OUT=/absolute/task-evidence/baseline-attempt01-ui-hotpaths.json \
"$CARGO_TARGET_DIR/release/examples/ui_hotpaths"
```

`CRAFT_UI_PERF_OUT` writes the pretty JSON report; unset it to emit JSON to stdout.
`CRAFT_UI_PERF_LABEL` is copied into the report. Default fonts are egui's bundled definitions;
optional `CRAFT_UI_PERF_FONT` supplies a pinned fixture file instead. Record its exact path and
SHA256 externally. The report's `fonts.font_file` is a readable string (null when unset); non-UTF-8
OS paths are displayed with replacement characters, so that display is not a lossless file identity.
Actual file loading retains the OS path. Keep raw reports, stderr, source/lock/executable hashes,
full argv/environment, Rust versions, feature set, CPU affinity and profile settings for both sides.
The report's `build_profile` uses `debug_assertions` as a label: it does not independently prove
optimization/LTO/codegen settings or account for environment profile overrides.

Schema 3 records the case matrix and raw arrays. Each case warms for at least 50 operations/frames
and at least 3 seconds. Pure functions collect 50 mean-of-batch samples (16 iterations per sample,
2,000 for split cases), plus 50 separate individual latencies; those individual micro latencies are
diagnostic because timer overhead dominates sub-microsecond work. Mean-of-batch samples are not a
frame-tail distribution. Dock/integration cases collect 1,000 individual frame durations with
median/p95/p99 statistics. Bodies, geometry, interaction checks, shapes/vertices/indices and the
64-bit mesh fingerprint come from the final measured frame; the fingerprint is not exact mesh or
pixel identity. Texture/repaint counters aggregate measured frames, and the hover probe runs in a
separate unmeasured frame. Galley counts are recorded after warmup and after measurement.

Before comparing paired reports, require nonempty consistent case keys/counts, matching viewport,
DPI, theme, font fixtures, features and instrumentation, and the intended reviewed dependency delta.
Alternate repeated runs and retain every raw pair; this example neither produces Criterion confidence
intervals nor automatically accepts regressions. Apply section 10.2's thresholds only to metrics the
actual output supports, with noise and correctness checks. Other criteria (exact visual/mesh equality,
allocation counts, a 10k-frame cache-growth run, printing/native behavior and GPU/upload timing) need
separate fixtures or profiling. Measure peak RSS/wall time around the built executable with the
parent's managed runner; they are not fields measured by this harness. A successful example run alone
does not establish performance, native rendering or all-platform acceptance.

### 10.2 Thresholds

These are acceptance thresholds for the migration, not automatically generated pass/fail golden
values. Correctness gates are hard. Performance thresholds apply after excluding correctness
failures.

| Metric | Threshold | Action on failure |
| --- | --- | --- |
| Exact visual pairs | 0 changed pixels, 0 changed mesh elements | Fix component or make a reviewed, justified reference change |
| Exact geometry fixtures | exact values | Fix component |
| Accessibility names/roles | exact | Fix component or scoped fixture policy |
| Idle repaint | 0 paints in 2 s after settle, no `request_repaint` from completed print state | Fix lifecycle |
| Dock frame median | <= +5% candidate vs paired baseline | Profile; rerun; no acceptance on one noisy run |
| Dock frame p95 | <= +10% | Profile; rerun; report |
| Pooled dock frame p99 (>=1,000 individual frames) | <= +15% | Profile; rerun; part of acceptance; max is diagnostic only |
| Layout pure functions median | <= +3% | Profile; rerun |
| Layout pure functions p95 | <= +5%, from individual latency samples | Profile; rerun |
| 800-action dock sequence median | <= +5%; inspect growth across larger trees | Inspect algorithmic regression |
| Texture upload bytes / 100 frames | <= +5%, no repeated upload of the same image | Inspect cache keys/URI |
| Peak RSS | investigate > +8 MiB or +5%, whichever is reached first | Report both absolute and percentage |
| Galley cache size | bounded, no monotonic growth over 10k frames | Inspect cache key/language/viewport |

Noise policy:

- Run baseline and candidate on the same host, same OS image, same Rust toolchain, same feature set,
  recorded lock identities and same viewport/DPI. Review the intended dependency delta rather
  than requiring identical old/new lock contents. Alternate baseline/candidate sessions; do not compare a
  laptop to the server or llvmpipe to a GPU.
- Warm up for at least 3 s or 50 iterations, whichever is longer. Discard warmup.
- Use 5 independent sessions of at least 200 individual frame samples for frame percentiles,
  plus the Criterion runs for microbenchmarks. If the coefficient of variation of medians is > 3%, or
  any session has a build/test/profiling process competing for CPU, discard and rerun the pair.
- Accept only when the median-of-medians, pooled p95 and pooled p99 meet the table.
  Report p99 only from the pooled >=1,000 individual samples, not each 200-frame
  session; if tail variability prevents a repeatable verdict, expand to 10,000 frames. Report the raw session
  values even when they pass.
- A changed visual golden is never auto-accepted because of a performance change. A compile-time
  measurement is not UI performance and must not be reported as such.

The Windows row is a separate performance table (Basic Render Driver, guest CPU, guest memory).
The macOS row requires real CI or hardware evidence; without it the row is unresolved. The real-GPU
row requires fresh adapter-qualified measurements. The suite protocol's startup counts, small
operation floor and stricter applicable thresholds also apply.

## 11. Implementation PR stages

Dependency DAG:

```text
all reviewed plans land
  -> CL-0 baseline harness and measurements (before production change)
winit completed integration including W15P + W27 lifecycle revision -> approved publication
  -> egui final P8 incl. P2b, P7, P7b/E21b and P7c -> approved publication
  -> CL-1 atomic API/fork graph
       -> CL-2 capture prototype + lazy-print tests
            -> approved prototype revision for integration consumers
            -> DesignCraft + GridCraft adapter PRs/reviews
            -> CL-2F final helper API freeze and docs
       -> CL-3 accessibility/glyph/persistence/direction tests
       -> CL-4 stock/fork fixtures and linked native acceptance
  -> CL-5 final compatibility documentation
```

CL-0 is a separate baseline harness stage. CL-1 cannot start until CL-0 baseline records are complete. CL-2 and CL-3 may proceed after CL-1
on disjoint code; their publication still requires approval. CL-2's initial API is
explicitly provisional until the two named adapters are reviewed, preventing a
dependency cycle between helper availability and API freeze.

### CL-1: egui 0.37 API and coherent suite lock

Files:

- `Cargo.toml` (workspace egui pin).
- `crates/ui/Cargo.toml` (dev dependency versions).
- `crates/ui/src/buttons/mod.rs`, `crates/ui/src/docking/surface.rs`,
  `crates/ui/src/layout/resize.rs`, `crates/ui/src/tabs/mod.rs` (role mapping).
- The exact closed test-file allow-list in §4.2 (`Id::unique`, `scope_id`, ellipsis fixture);
  additional files need a named reviewed exception, not a broad glob.
- `crates/ui/README.md` (version + `scope_id` example).
- Root patches and `Cargo.lock` regenerated against the reviewed immutable storytold egui/winit
  commits; retained PDF backend patches remain intact.

Use a local stock-0.37 experiment to isolate API fallout, then land the coherent suite graph in
one compatibility PR after the fork dependencies are published with approval. Stock compatibility
is verified independently; no intermediate registry-only consumer graph is a suite handoff.
Rollback: revert the coherent commit. The old coherent set is preserved at
`49190884278f16c9a9f5823d7698a8ffb5deb062`: egui family 0.36.2, accesskit 0.24.1, kittest 0.4.0,
winit 0.30.13 and the baseline `Cargo.lock`. Do not partially revert only one crate and do not leave a
0.37 manifest with a 0.36 lock.

Prerequisite: the completed reviewed/published P8 fork includes P2b, P7, P7b/E21b and
P7c, with W15P and the W27 lifecycle revision. Intermediate pins are not accepted.
Acceptance: all section 9.1 gates against the suite fork and focused stock-0.37 compatibility
checks; `cargo tree` has one egui family; visual
suite still exact; no deprecation warnings under clippy `-D warnings`.

### CL-2: Capture completion and existing lifecycle contracts

Files:

- `crates/ui/src/lib.rs` declares public `capture`; the lightweight slot has no new feature
  gate or renderer dependency. Public types are available under `craft_ui::capture`.
- `crates/ui/src/capture.rs` completion-slot prototype and tests. Publish only after
  approval as a provisional git API; DesignCraft and GridCraft then implement their
  adapters against that exact revision. CL-2F freezes the public API after joint review.
- `crates/ui/src/printing.rs` only if a demonstrated regression needs a fix; preserve lazy discovery.
- `crates/ui/src/printing/tests.rs` (logic-only and one-worker tests).
- `crates/ui/README.md` lifecycle paragraph.

No change to `Settings`, `Feedback`, `Discover`, `Submit`, `begin`, `ready`, or job ownership.
Rollback: revert the helper together with its adapters, preserving pending-request cleanup and
the previously coherent app dependency set. Print behavior changes require their own regression.

Acceptance: tests from sections 6.2 and 7.8; cancellation, out-of-order result and shutdown stress;
no worker before print-open, no duplicate workers or repaint after completion.

### CL-3: Accessibility, glyph, persistence and docking direction contracts

Files:

- `crates/ui/src/docking/tests.rs`, `tabs/tests.rs`, `layout/tests.rs`, `buttons/tests.rs`,
  `printing/tests.rs`.
- `crates/ui/src/docking/surface.rs` implements `DockArea::direction`/`DockDirection`;
  `crates/ui/src/docking/mod.rs` re-exports the public direction type.
- New `crates/ui/tests/panel_integration.rs` and `serde_contract` test module;
  opt-in docking direction implementation and the exact geometry/hit/drag tests in §7.7.
- Beyond the explicit direction policy, no production change unless an assertion exposes a real component bug; then fix the component in
  the same PR and document the exact role/name change.

Acceptance: exact roles/names as section 7.2; `Harness::check_accessibility`; glyph coverage; fixed
JSON round-trip; panel order/clip/float layer; targeted failure probes. Rollback: revert the
production direction API and its app adapters as a coherent set; other fixtures are test-only.
Any additional component fix is identified and reverted separately.

### CL-4: Suite fork, winit pin and dual-compatibility checks

Files:

- Verify the root patches and lock introduced atomically in CL-1, with one git source per family.
- `ci/registry-egui` isolated workspace plus `cargo package -p craft-ui` smoke check.
  This preserves library portability; it does not authorize a crates.io release.
- `ci/egui-fork-contract` isolated workspace that depends on the four UI-facing craft crates and
  compiles against the fork with `eframe` wgpu/accesskit/x11/wayland and winit.
- Documentation of the consumer patch requirement (README or docs).

This PR does not publish anything and does not change the published manifest's registry requirement.
Rollback: retain CL-1's coherent dependency set and revert only this stage's fixture changes.

Acceptance:

- `cargo tree -d` empty (or only pre-existing unrelated duplicates documented);
- exactly one `egui`/`epaint`/`egui-winit` and one `winit` source;
- fork contract compiles on Linux (x11+wayland), Windows GNU check, and wasm check for `craft-ui`;
- packaged `craft-ui` still compiles/tests against registry 0.37.0;
- E1/E4/E19/E21 symbol smoke checks compile only where the API exists;
- no branch names in locks or manifests.

### CL-5: Documentation and release checklist

Files:

- `crates/ui/README.md` compatibility paragraph: registry 0.37 vs suite fork, consumer patch
  requirement, `scope_id` example.
- `docs/ui-components-audit.md` version statement and a dated migration note; do not rewrite its
  historical pinned hashes.
- `docs/egui-037-migration-plan.md` status update only after the plan is approved.
- Release notes in the user's voice, no agent/model/tool attribution.

Acceptance: documentation links to immutable commits/tags; no machine-local path; no claim of a
performance or platform result that was not measured. Rollback: documentation-only.

## 12. Risks and migration traps

1. **Lock conflict regression.** The audit merge proves how easily a 0.37 manifest gets a 0.36 lock.
   Always inspect the lock, not only the manifest, and run `cargo tree`.
2. **Fork patch not propagating.** A consumer that only patches `egui` will get registry `egui-winit`
   or registry winit. The root patch must name every family member and winit.
3. **Movable branch pins.** `craft/0.37`, `craft/0.30.13-lang` and `origin/ai-0.37` can move. Use full
   SHAs in all locks, fixtures and docs.
4. **Fork vs upstream API leak.** The shared library deliberately preserves stock 0.37 compatibility,
   independently of its git-only suite delivery. Keep fork-only APIs in app/fork
   fixtures: `Context::set_language`, `UiBuilder::language`, `Response::accessible_language`,
   `Frame::set_ui_language`, `LayoutJob::{language,direction}`, `LineBreaker`,
   `FontTweak::fallback_align`, `Style::right_to_left`, `RawInput::latin_layout_chars`,
   `RawInput::keyboard_layout_is_latin`, E21b surrounding-text/owner payload and clear APIs,
   `ImeEvent::DeleteSurroundingBytes` and `ViewportHooks`. The independent stock
   compile/test fixture is authoritative, not just a symbol-name grep.
5. **Default fonts changed twice.** `default_fonts` became opt-in and monochrome emoji fonts became
   separate. The old `⋮` tofu case became a hard test failure under the new panic policy. Custom app
   fonts, not craft-ui production defaults, must prove glyph coverage.
6. **Kittest panic policy.** `MissingGlyphPolicy::Panic` and `check_accessibility` are deliberate bug
   detectors. Scoped opt-outs, never global.
7. **Screenshot callback ownership.** Callbacks are one-shot, `Send + 'static`, and execute outside
   the UI pass. A callback that captures `&mut App` will not compile; a callback that never wakes the
   app will strand a hidden-window result.
8. **Surface fitting changes screenshot pixels.** E2 may render a window at a smaller surface and
   scale it. Use returned image dimensions and capture-time logical viewport/DPI
   metadata; the private integration render_scale is not a public app API.
9. **Manual DockArea layout is not mirrored by `Style::right_to_left`.** Tabs, split first/second and
   floating grip placement follow the explicit opt-in visual policy in section 7.7;
   E9 does not implement it automatically.
10. **Bidi changes text widths.** A tab label with Arabic/Hebrew can change `tabs::fit` results. Keep
    ASCII fixtures exact and add fork-specific mixed-script geometry tests; do not update ASCII
    references for an unrelated reason.
11. **IME and undo.** E21's exact-byte deletion is only correct if the app keeps the commit and
    deletion in one undo transaction. Do not reimplement deletion in app code.
12. **Clipboard PRIMARY.** Regular copy/paste and middle-click PRIMARY are separate paths and separate
    compositor behaviors. Test both.
13. **GPU/resource tests.** `WgpuTestRenderer` is offscreen and prefers CPU adapters but may fall back to
    a physical GPU. Record adapter identity; enforce a CPU adapter for deterministic
    goldens and keep physical-GPU performance rows separate. It is not native
    focus/IME/picker evidence and not a real-GPU claim.
14. **Panel/root UI borrow.** Calling `Context::root_ui` from inside `DockArea` or a component panics.
    Components take `&mut Ui`.
15. **Multiple passes.** `request_discard` can repeat a `run_ui` closure. One-shot side effects in
    tests or app callbacks must be guarded.

## 13. Resolved design and blocking acceptance

1. **Dependency policy.** Integrate the reviewed patches into storytold egui/winit first; pin
   their final immutable commits at the suite root. The aiongg SHAs are reviewed inputs, not
   promised production tips. A separate registry fixture verifies stock API compatibility.
2. **Versioning.** Ship a new git revision and compatibility note; no blanket workspace version
   bump or crates.io publication is part of this task.
3. **Printing.** Keep user-triggered/lazy `refresh` and idempotent `poll`; hidden polling must
   deliver requested results without starting unrelated printer discovery.
4. **Docking direction.** Preserve serialized panel IDs, split axes, tab order and positions.
   Adopt the explicit opt-in DockArea direction policy in §7.7; test RTL painting,
   hit coordinates, visual-to-logical drag insertion and unchanged serialization. Do not silently mirror the persisted model or add a locale redesign.
5. **Capture.** Use the narrow weak completion-slot design from section 7.8; review two concrete
   callers and stress cancellation/memory before freezing the API. Keep app policy outside it.
6. **Platform evidence.** Qualify the accessible Linux i915 render node and software backends
   separately. Windows Basic Render Driver is software evidence. macOS needs native evidence,
   and wasm compilation does not replace a real browser interaction workflow.

Blocking acceptance criteria for craft-libs:

- CL-1 through CL-4 gates pass with fresh task/evidence records; no direct cargo run.
- One egui family and one winit source in the fork lock; packaged crate still resolves stock 0.37.
- Fresh reports reproduce or rule out the applicable upstream #8731 text-tessellation and #8512
  font-startup regressions; renderer fixtures include #8736 DX12 setup, #8763 selection/action and
  hidden/deferred viewport behavior. #8763 is the open selection issue; #8764 its open, unmerged
  proposed fix. #8748 is a closed, unmerged PR. #8512 is a closed feature PR whose comments supply
  the startup-cost discussion. None is evidence of a released regression fix.
- Exact visual/geometry/accessibility/persistence tests and the deliberate failure probes pass.
- `cargo tree -d` has no new duplicates; no branch refs; no machine-local paths.
- The performance table meets the thresholds, or a regression is profiled and explicitly reviewed
  with a documented exception. A compile-time number, a changed golden or a single noisy sample is
  not acceptance.
- Every push/new PR has explicit human approval after the exact diff and validation are shown.
  Commits and PR text use the user's voice and username-only contributor credit (`@skylartaylor`);
  no agent/model/tool attribution.

## 14. Source index used for this plan

Primary upstream:

- egui 0.37.0 changelog and release commit `841a328b`: `CHANGELOG.md` at tag `0.37.0`.
- `crates/egui/src/context.rs` (`run_ui`, `run_pass`, `root_ui`, `run_logic`, `request_screenshot`).
- `crates/egui/src/id.rs` (`Id::unique`, `IdSalt`), `ui.rs` (`scope_id`), `ui_builder.rs`.
- `crates/egui/src/containers/panel.rs` (`Panel`, `CentralPanel`).
- `crates/egui/src/data/screenshot_callback.rs`, `viewport.rs` (`Screenshot`).
- `crates/egui_kittest/src/builder.rs`, `accessibility.rs`; `crates/epaint/src/text/fonts.rs`
  (`MissingGlyphPolicy`, `has_glyphs`).
- Upstream PRs: #8361, #8428, #8489, #8534, #8535, #8542, #8555, #8577, #8581, #8583, #8589, #8592,
  #8601, #8617, #8657, #8641, #4729 (winit).

Suite fork:

- `/home/codex/work/CraftApps/egui037-migration-20261022/egui`:
  `aiongg/craft/0.37` = `a4f9a17f64a6bae61350fb85804c9df0d9865916`;
  `PATCHES.md` E1-E21; `0.37.0` = `841a328b`.
- `/home/codex/work/CraftApps/craft-fork-20261020/winit-ai-lang`:
  `craft/0.30.13-lang` = `8b35388bd719e8cc86a95971ba0bc026c9a8f058`;
  `craft/0.30.13` = `ffae5198`; PATCHES.md W1-W27.
- craft-libs PR 199 ref `refs/review/pr199` = `b68de8c`; audit merge `ec8840a` is not a target.
- Historical evidence, not fresh validation:
  `/home/codex/work/ssd2/task-evidence/craft-fork-plan-20261020/PLAN.md`,
  `/home/codex/work/ssd2/task-evidence/craft-fork-ai-lines-20261022/SURVEY.md`,
  `/home/codex/work/ssd2/task-evidence/craft-fork-egui037-20261021/RESULTS.md`,
  `/home/codex/work/ssd2/task-evidence/egui037-audit-20261022/fork/craft-libs.log`.

craft-libs source anchors:

- `Cargo.toml:34`; `crates/ui/Cargo.toml:13-24`; `.github/workflows/ui.yml:20-58`.
- `crates/ui/src/lib.rs`; `crates/ui/src/buttons/mod.rs:70-102`.
- `crates/ui/src/tabs/mod.rs:40-163`.
- `crates/ui/src/layout/geometry.rs:1-135`; `layout/resize.rs:1-198`; `layout/toolbar.rs:1-152`.
- `crates/ui/src/docking/model.rs:1-798`; `docking/surface.rs:1-919`; `docking/targets.rs:1-99`.
- `crates/ui/src/printing.rs:1-261`; `printing/tests.rs:1-257`.
- `crates/ui/src/theme/mod.rs:1-97`; `icons/mod.rs:1-78`.
- `crates/ui/tests/visual_regression.rs:1-394`; `tests/visual/fixtures.rs:1-327`;
  `tests/docking_workflows.rs:1-252`.
- `crates/platform/src/appearance.rs:439-525`; `platform/src/dialogs.rs:220-251`;
  `crates/quit/src/close.rs:1-190`; `crates/window/src/lib.rs:255`.
- `crates/ui/README.md:115,237`; `docs/ui-components-audit.md:5`;
  `docs/ui-visual-regressions.md`; `docs/ui-docking.md`.


## Local implementation update — 2026-10-23

The user clarified that reviewed plans remain local and implementation proceeds before publication. The earlier requirement to publish/land this plan first is superseded; pushes and PR publication still require explicit approval.

Canonical `storytold/craft-libs` `main` now resolves to `f180fc0d35d521c5d18e118c881d3efde1ec1eee`. The owned checkout was fast-forwarded to this revision. Earlier baseline SHAs and line anchors above are retained as historical review evidence; use this new revision and its exact lockfile for current-main integration and paired measurements. The original refs remain reachable.

Preserve new hayro/krilla hostile-input bounds, updated SOURCE manifests/nested lock, and chrono std-only workspace policy. PR199 advanced to1dcde7ed and now incorporates49190884 security fixes; its14-file delta can be reviewed against currentmain rather than treating newhead as the old stale tree. Regenerate lock against final dependencies.

The complete static delta review and exact old/new object identities are retained in the task evidence: `canonical-diffs-refresh-20261023.json` and `overnight-consumer-review-20261023.md`. This amendment claims no new build, native GUI or performance acceptance.

### Local row-alignment component, 2026-10-23

`craft_ui::text::align_galley_rows` positions already-shaped rows within an explicitly allocated width. It accepts a row alignment callback and uses stock egui 0.37 types, so it does not make the shared UI crate depend on the fork's bidi extensions. It is useful for comment cards, captions and message panels. It retains shared row glyphs/meshes and logical text, updates row positions and logical/mesh bounds, and never mutates the cached source galley. Oversized left-aligned rows overflow right, right-aligned rows overflow left, and centered rows overflow on both sides; nonfinite or negative widths are refused without changing the input.

WordCraft supplies alignment from the fork's `rtl_paragraph` field. This preserves native shaping, mixed-direction runs, mirrored bracket glyphs and neutral-paragraph inheritance. The fork's `LayoutJob::halign_min_is_start` aligns RTL rows to the widest text row, which is insufficient for a short comment in a wider card. No new Unicode reordering implementation is introduced. Unicode's logical-order and mirroring rules are described in [UAX #9](https://www.unicode.org/reports/tr9/); the fork source documents its row placement in `epaint/src/text/text_layout_types.rs`.

Prepared helper tests check independent row alignment, unchanged cached row data, mesh bounds, invalid widths and oversized rows. WordCraft's real Arabic/mixed/neutral-paragraph test passes in `egui037-wordcraft-candidate-20261023-targeted-07`, including atlas-bitmap bracket orientation. Stock shared-library tests, performance and independent component review are pending; the prior 110-test result predates this helper.

The second independent component review found no blocking helper defect. Word now clips balloon ink to the card and disables the fork-only start-edge override before custom row alignment. Additional source tests cover fractional scale, zero width, idempotence and separate ink bounds. Execution remains pending.
