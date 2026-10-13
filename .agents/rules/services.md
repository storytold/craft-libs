# Shared Services & Architecture Subsystems

This document defines the architecture, responsibilities, and operational contracts for shared services across `craft-libs`.

---

## 1. Atomic Persistence Service (`craft-atomic`)

`craft-atomic` provides crash-resilient file replacement, guaranteeing that documents and database states are never left half-written or corrupted by process crashes or system interrupts.

### The Persistence Protocol
Atomic file replacement follows a strict sequence:
1. **Isolated Staging**: Write data to an ephemeral sibling temporary file:
   ```text
   <target_path>.tmp.<pid>.<monotonic_counter>
   ```
2. **File Data Sync**: Flush file buffers to the operating system kernel (`sync_data` or `sync_all`).
3. **Atomic Rename**: Atomically rename the staging file over the destination file.
4. **Parent Directory Sync**: Flush the parent directory to commit the directory entry update to storage.

### Core Invariants
- **Ordering vs. Hardware Durability**: The sequence `sync_file < rename < sync_dir` guarantees proper syscall dispatch and kernel ordering. It does not guarantee physical disk platter flush on unbuffered controllers or power interruption without battery backup.
- **Permission Preservation & New-File Policy**: On overwriting an existing file, the new file must inherit and preserve the original file's POSIX mode/permissions. For new files, the caller explicitly selects `PlatformDefault` (0666 minus umask/ACL) for standard documents, or `Private` (0600) for internal configuration and recovery journals.
- **Cross-Device Recovery**: If the destination path is on a different filesystem or mount point (`EXDEV`), fall back to copy-then-unlink with safe cleanup of the temporary staging file.
- **Symlink Protection**: Never silently follow destination symlinks that masquerade as regular files.

---

## 2. Configuration & State Storage Service (`craft-settings`)

`craft-settings` manages persistent application settings, workspace layouts, and recent file records in structured TOML format (`config.toml`).

### Backup Preservation & Recovery Protocol
1. **Multi-Level Pre-Validation**: Before replacing an existing backup (`config.toml.bak`), the active file must pass two validation layers: well-formed TOML parse and valid schema version bounds.
2. **Never Overwrite with Corruption**: If the active configuration is empty, truncated, or unparseable, the existing `.bak` file is preserved untouched.
3. **Diagnostic Move-Aside**: Corrupt primary files must never be discarded silently. Move them aside with a Unix timestamp for diagnostics (`config.broken-<unix-timestamp>.toml`).
4. **Multi-Stage Recovery**: When launching with damaged primary settings, attempt automatic restoration from `config.toml.bak` before resetting to factory defaults.

---

## 3. Platform Lifecycle & Termination Service (`craft-quit`)

`craft-quit` manages application shutdown, dirty-document lifecycle confirmations, and native menu bar integration.

### Core Contracts
- **Native macOS Menu Architecture**: Menu bar initialization is owned by `craft-menu` / `craft-quit`. `muda::MenuEvent::set_event_handler` is backed by a process-wide `OnceLock`. Calling competing menu installers causes collisions and is strictly forbidden.
- **Dirty Document Confirmation Protocol**: Never terminate an application silently when unsaved documents exist. Applications register confirmation handlers to prompt users before quit execution.
- **Event Loop Wakeup**: All menu activations and accelerators must trigger `repaint.request_repaint()` to eliminate event loop dormancy.

---

## 4. Immediate-Mode UI Service (`craft-ui`)

`craft-ui` provides pure, reusable presentation components for egui frontends without pulling in application domain logic.

### Core Contracts
- **Layer Independence**: Pure presentation code sits strictly at the frontend layer. Shared components accept small style values (`WidgetPalette`, `IconButtonStyle`) and return interactions (`egui::Response`, typed action events).
- **Upstream-First UI Pre-Check**: Before authoring presentation widgets in application repositories, inspect `docs/ui-elements-map.md` and `docs/ui-schema.md`. Shared widgets belong in `craft-ui` to eliminate duplicate implementations across the suite (Issue #67).
- **Single Commit Undo Granularity**: Continuous drag interactions (scrubbing, zooming, sliders) stream intermediate preview values to callers, but emit exactly one commit transaction on pointer release.
