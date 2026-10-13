# Negative Results and Disproven Designs

Status: Canonical repository record.  
Rule: Disproven hypotheses, rejected architectures, and incompatible benchmarks are recorded permanently to prevent re-proposing abandoned directions.

## 1. Universal Non-Negative Media Time Clamping

- **Proposed**: Clamping `Tick` to non-negative `[0, MAX]` universally inside shared library crates.
- **Disproven**: Breaks audio lead-in, music pickup bars, negative trim offsets, and negative timeline margins in FilmCraft, EffectCraft, and SoundCraft.
- **Adopted Invariant**: `Tick` maintains its signed arithmetic contract across `[Tick::MIN, Tick::MAX]`. Non-negative bounds belong strictly at application-level playback or seek boundaries.

## 2. Leaked Static Slices for SVG Catalogs

- **Proposed**: Leaking prepared byte buffers via `Box::leak` to obtain static slices (VectorCraft approach).
- **Disproven**: Produces unrecoverable memory leaks when icon catalogs change or reload.
- **Adopted Invariant**: Shared owned bytes held in `Arc<[u8]>` with fallback resolution before URI source generation (`craft-ui::icons`).

## 3. Monolithic Universal Theme Tokens Union

- **Proposed**: Combining all 13 application theme fields into a single master `Tokens` struct.
- **Disproven**: High coupling; forces applications to carry unused fields and schema migrations for unrelated domains.
- **Adopted Invariant**: Small, focused widget-state styling via `theme::WidgetPalette` and `IconButtonStyle`. Applications retain their own complete palettes and token schemas.

## 4. Unvalidated Settings Backup Replacement

- **Proposed**: Overwriting `config.toml.bak` on every settings flush.
- **Disproven**: Corrupted active settings files overwrite good backups, causing irrecoverable configuration loss.
- **Adopted Invariant**: Multi-level pre-validation (syntax parse + schema version bounds) before replacing backups. Damaged primary files are moved aside for diagnostics (`config.broken-<unix-ts>.toml`).

## 5. Silent Parent Directory Creation on Atomic Document Replacement

- **Proposed**: Automatically creating missing parent directories (`create_dir_all`) inside `craft-atomic::replace_file`.
- **Disproven**: In PhotoCraft and VectorCraft, saving to unmounted network volumes or missing drives silently created local directories on the root volume, leading to ghost saves and lost user documents.
- **Adopted Invariant**: `replace_file` rejects missing parent directories with `ParentFolderNotFound`, requiring callers to explicitly declare intent if directories should be created.

## 6. Child Process Spawning for Windows Printing

- **Proposed**: Discovering Windows print queues and submitting jobs via spawned `powershell.exe` child processes.
- **Disproven**: High latency (multi-second delays per invocation), execution policy blocks on locked-down enterprise machines, antivirus false positives, and unnecessary process overhead.
- **Adopted Invariant**: In-process native Win32 GDI and print spooler APIs via `craft-print` without spawning external shell processes.

## 7. Direct Process Exit on macOS Termination (`Cmd+Q`)

- **Proposed**: Calling `std::process::exit` or unconditional window closing when receiving `Cmd+Q`.
- **Disproven**: Bypasses AppKit Cocoa window delegation, aborting background file synchronization, leaving temporary files in place, and failing to prompt for dirty document saves across open windows.
- **Adopted Invariant**: Termination routed through `NSApplication terminate:` via `craft-quit`, allowing native multi-window save prompts and user cancellation.

## 8. Hardcoded Restrictive File Permissions (`0o600`) for All Atomic Writes

- **Proposed**: Setting `0o600` (user read/write only) unconditionally on every newly written atomic file.
- **Disproven**: Breaks collaboration, group workflows, export files, and command-line interoperability across the Craft applications.
- **Adopted Invariant**: New files default to platform umask and standard permissions, with explicit `write_new_private` opt-in strictly reserved for credentials and secrets (PR #43).

## 9. Mutually Exclusive Menu Event Handlers in `muda`

- **Proposed**: Allowing `craft-quit` and `craft-menu` to both install independent menu event listeners via `muda::MenuEvent::set_event_handler`.
- **Disproven**: `muda` backs its event handler with a process-wide `OnceLock`. Calling `set_event_handler` a second time panics immediately at runtime.
- **Adopted Invariant**: Single-owner pattern where `craft-menu` exclusively owns `muda` initialization and event dispatch, while `craft-quit` remains decoupled, owning application termination and dirty document lifecycle logic.

## 10. Synchronous Filesystem and Threading on WebAssembly

- **Proposed**: Compiling synchronous POSIX filesystem calls (`fsync`, tempfiles, locking) directly on `wasm32-unknown-unknown`.
- **Disproven**: Triggers runtime traps, missing symbol link errors, and compilation panics (Issue #81).
- **Adopted Invariant**: Strict `#![cfg(not(target_arch = "wasm32"))]` boundaries with non-panicking, fallible in-memory fallbacks on WebAssembly targets.

## 11. Direct Working Tree Mutation in Compatibility Runner (`xtask compat`)

- **Proposed**: Mutating the developer's working directory or modifying committed `Cargo.toml`/`Cargo.lock` during compatibility validation runs.
- **Disproven**: Dirties local git status, leaves broken lockfiles on test aborts, and prevents concurrent development.
- **Adopted Invariant**: Completely isolated ephemeral worktrees created via `git worktree add` with temporary indexes (`GIT_INDEX_FILE`), preserving developer working tree state.

## 12. In-Tree Ad-Hoc UI Component Duplication

- **Proposed**: Each application authoring its own presentation controls (ZoomBar, rulers, color pickers, splitters) in local UI crates.
- **Disproven**: Architectural fragmentation, conflicting keyboard and focus accessibility models, and redundant maintenance overhead across 13 apps (Issue #67).
- **Adopted Invariant**: Upstream-first UI protocol: developers must inspect `docs/ui-elements-map.md` and `docs/ui-schema.md` before authoring in-tree widgets.

## 13. Hardware Crash Durability Inferred from Filesystem Ordering Alone

- **Proposed**: Claiming physical power-loss crash durability based on software `sync_file < rename < sync_dir` ordering.
- **Disproven**: Disk controller write caches, unbuffered drive firmware, and virtualized storage layers may reorder or drop flushed blocks on power interruption without explicit hardware verification.
- **Adopted Invariant**: Clear claim taxonomy: filesystem sequence proves kernel dispatch and ordering, not hardware crash durability across physical power drops (PR #24).
