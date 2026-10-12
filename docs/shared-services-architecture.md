# Shared Services Architecture

This document defines the architecture and contracts for shared services implemented across `craft-libs`.

---

## 1. Atomic Persistence Service (`craft-atomic`)

`craft-atomic` provides crash-resilient file replacement, ensuring documents and settings files are never corrupted by mid-write crashes or interrupted flushes.

### The Persistence Protocol

Atomic file replacement follows a strict four-stage protocol:

1. **Staging File Creation**: Write data to a sibling temporary file:

   ```text
   <target_path>.tmp.<pid>.<monotonic_counter>
   ```

2. **File Data Sync**: Flush file buffers to the operating system kernel (`sync_data` or `sync_all`).
3. **Atomic Rename**: Atomically rename the staging file over the destination file.
4. **Parent Directory Sync**: Flush the parent directory to commit the directory entry update.

### Contracts & Invariants

- **Ordering vs. Hardware Durability**: The sequence `sync_file < rename < sync_dir` guarantees syscall dispatch and kernel ordering. It does not claim unbuffered physical disk power-loss survival.
- **Permission Preservation**: Overwriting preserves the destination file's original POSIX permissions.
- **Cross-Device Recovery**: If destination is on another filesystem (`EXDEV`), fall back to copy-then-unlink with safe staging file cleanup.
- **Caller Intent Preservation**:
  - *PhotoCraft*: Document Save/Save As requires existing parent directories to prevent silent directory creation on offline volumes; headless and export flows permit explicit creation.
  - *VectorCraft*: Preserves `write_new_with` semantics, directory sync qualification.
  - *LightCraft*: Distinguishes durable document state (requires sync) from high-throughput recreatable cache data (omits mandatory sync).

---

## 2. Configuration & State Storage Service (`craft-settings`)

`craft-settings` manages structured TOML application configuration (`config.toml`).

### Backup Preservation & Recovery Protocol

1. **Multi-Level Pre-Validation**:
   Before replacing an existing backup (`config.toml.bak`), the active file must pass two validation layers:
   - Syntactic TOML parse (must be well-formed, non-empty TOML).
   - Schema bounds check (known schema version within supported bounds).
2. **Never Overwrite with Corruption**:
   If the active configuration is empty, truncated, or unparseable, the existing `.bak` file is preserved untouched.
3. **Diagnostic Move-Aside**:
   Damaged primary files are preserved with a Unix timestamp for diagnostics:

   ```text
   config.broken-<unix-timestamp>.toml
   ```

4. **Multi-Stage Recovery**:
   When launching with damaged primary settings, attempt automatic restoration from `config.toml.bak` before falling back to defaults.

---

## 3. Platform Lifecycle & Termination Service (`craft-quit`)

Manages clean application termination signals across desktop operating systems:

- **macOS Integration**:
  - Hooks into AppKit lifecycle (`applicationShouldTerminate:`).
  - Routes menu bar "Quit" (`Cmd+Q`) and Dock menu quit actions through the application's unsaved document check.
  - Physical Touch Bar and menu quit operations must not trigger unhandled native crashes.
- **Windows & Linux**:
  - Handles `WM_CLOSE` on Windows and `SIGINT`/`SIGTERM` on POSIX systems.
- **Buffer Flushing**:
  - In-flight staging buffers and asynchronous disk writes are flushed cleanly before process exit.

---

## 4. UI & Presentation Services (`craft-ui`)

Headless-compatible UI primitives built on `egui` and `wgpu`:

- **Docking Workspaces**: Tab strips, accordion containers, split panes, and floating tool windows with keyboard navigation and versioned TOML layout serialization.
- **Theme Infrastructure**: Design tokens, dark/light palettes, and high-contrast accessibility borders.
- **Icon Asset Cache**: On-demand SVG rasterization and caching.
