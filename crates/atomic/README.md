# Craft Atomic

`craft-atomic` provides crash-safe atomic file writing and replacement for the Craft suite.

Extracted from the storage engine in **PhotoCraft** (refs #2, #14), it guarantees that a failed save or abnormal termination never leaves user documents truncated, corrupted, or missing.

## Core Guarantees: Atomicity vs. Durability
 
1. **Atomicity (All-or-Nothing Replacement)**: `atomic_write` never truncates or overwrites the destination in place. It stages content in a hidden temporary file beside the destination (`.{stem}.{pid}-{n}.craft_tmp`), guaranteeing same-volume rename semantics (`EXDEV`-free). An external reader never sees a partial write. If an error occurs during staging or rename, the destination retains its prior bytes and the staging file is unlinked in user space. An abnormal process exit (e.g. `SIGKILL`, crash, or power loss) bypasses user-space cleanup and leaves the staging file on disk; sweeping orphaned staging files is delegated to consuming application recovery routines. The destination file itself is never corrupted.
2. **Durability & Ordering Boundaries**:
   - **Content Flush**: Before rename, staging file bytes are flushed to physical storage media (`sync_file`, issuing `F_FULLFSYNC` on macOS/APFS with SMB `fsync` fallback).
   - **Directory Flush (Unix)**: After rename, syncing the parent directory table (`sync_dir`) is attempted on a best-effort basis. If directory sync fails, the error is ignored because the rename has already committed and the destination has already been replaced.
   - **Hardware Durability Limits**: The sequence `sync_file < rename < sync_dir` proves ordering and kernel dispatch. Software tests do not prove survival across physical power interruption or unbuffered drive controller write caches.
3. **Transient Lock Backoff (Windows)**: Antivirus scanners and search indexers hold newly written files momentarily. `RenameRetry` automatically retries renames encountering `PermissionDenied`, sharing violations (32), or lock violations (33) with exponential backoff capped at 200ms.
4. **Permission & Symlink Invariants**: On Unix, new documents and exports use normal creation permissions (`0666`, subject to umask/default ACL), including their staging files. Private data must explicitly select `NewFilePermissions::Private` to create staging files with mode `0600`. Replacements always stage privately and restore the existing destination mode after the streaming callback. New files keep callback-selected modes with both hard links and the exclusive-copy fallback. Symlink chains are resolved with a hop limit, writing through the symlink to the ultimate target without replacing the link itself.
5. **Safe Exclusive Creation (`write_new`)**: Uses hard links where supported, or exclusive create (`O_EXCL`), guaranteeing that an existing file is never replaced under race conditions.
6. **Fault-Injection Testability**: The `AtomicFs` trait enables unit and integration test suites to simulate disk-full, locked files, and permission failures without physical disk I/O.

A replaced file is a new file (a new inode), so its owner and group, extended attributes and ACLs, Finder tags, alternate data streams and hard links are not kept; a crash between the write and the rename can leave a `.<name>.<pid>-<n>.craft_tmp` file beside the destination.

## Usage

### One-Shot Atomic Write

```rust
use std::path::Path;
use craft_atomic::atomic_write;

let path = Path::new("project.craft");
let data = b"precious document state";

// Atomically replaces project.craft. If writing fails, project.craft retains
// its prior bytes and temporary staging files are unlinked in user space.
atomic_write(path, data).expect("safe atomic write");
```

### Streaming Atomic Writer

```rust
use std::io::Write;
use std::path::Path;
use craft_atomic::atomic_write_with;

let path = Path::new("large_project.craft");

atomic_write_with(path, |file| {
    file.write_all(b"header...")?;
    file.write_all(b"payload...")?;
    Ok(())
}).expect("streaming atomic write");
```

### Exclusive Creation (`write_new`)

```rust
use std::path::Path;
use craft_atomic::write_new;

let path = Path::new("export.png");

// Writes only if the destination does not already exist; fails with AlreadyExists
// without modifying existing content.
write_new(path, b"png bytes...").expect("exclusive write");
```

### Document Permissions and Private Data

The free functions and `WriteOptions::default()` use `NewFilePermissions::PlatformDefault`.
On Unix, a new document/export stage is created directly with mode `0666`; the OS applies the
process umask, setgid directory group and default ACL. This matches ordinary file creation
(for example, `0644` under umask `022`, or `0664` under `002`). No process-global umask changes,
empty permission probe, or `chmod` is needed for an unchanged new-file mode. The same policy
applies when exclusive creation falls back to copying on volumes without hard links.

This is a deliberate change from the earlier untagged revision's private default. Settings,
credentials and recovery data must opt into private creation **before writing any bytes**:

```rust
use std::path::Path;
use craft_atomic::{NewFilePermissions, WriteOptions};

fn save_private_data() -> std::io::Result<()> {
    let private = WriteOptions::default().with_new_file_permissions(NewFilePermissions::Private);
    private.atomic_write(Path::new("settings.toml"), b"theme = 'dark'")?;
    private.write_new(Path::new("recovery.bin"), b"recovery bytes")
}
```

`WriteOptions` and `NewFilePermissions` are non-exhaustive: start with `default()` and select
the policy with the builder, so future options do not break callers. Downstream matches on the
enum must include a fallback arm. Byte and streaming methods support both policies.
`craft-settings` explicitly chooses `Private` for its config and backup writes.

Normal stages can be readable (or writable) by others allowed by the directory, umask and ACL
while the callback is running, and can remain after a crash. Choose `Private` up front for
sensitive payloads; changing permissions inside a callback is too late to guarantee that no
other process opened the stage. Private stages request `0600`; actual enforcement depends on
the filesystem's permission support. Neither policy adds Windows ACL guarantees.

Replacements still stage privately and restore the existing Unix mode after writing. When the
stage already has that mode, no `chmod` is attempted. This lets replacements work on fixed-mode
GVfs/SMB mounts and document portals that reject permission changes even when the reported mode
already matches. The comparison happens after the callback, so its mode changes are still
restored. A new file keeps any mode selected by its callback. In the normal exclusive-copy fallback, a changed
mode is applied before copying bytes; explicit private copies stay private during copying and
apply callback-selected modes afterward. A required mode change that fails is reported, not
silently ignored. Existing-file replacements and deliberate callback changes can therefore
still require `chmod`; this is not a claim that every save works on every external filesystem.

These policies do not promise preservation of existing ACLs, ownership or other metadata
across replacements. The copy fallback never overwrites an existing target, but a reader may
see the newly created target while it is being copied. A failed copy is removed.

### Fault Injection & Testing

`AtomicFs` lets callers simulate disk-full, locked files and permission failures. Its
`create_new(path, permissions)` operation receives the selected policy; there is no separate
probe operation or unsupported-probe fallback. Custom implementations migrating from an earlier
untagged revision must accept the policy argument and implement both creation modes.

The options API supports the same injection for byte and streaming writes:

```rust
use std::path::Path;
use craft_atomic::{AtomicFs, NewFilePermissions, RenameRetry, WriteOptions};

fn save_with_faults(fs: &dyn AtomicFs, path: &Path) -> std::io::Result<()> {
    let options = WriteOptions::default().with_new_file_permissions(NewFilePermissions::Private);
    let retry = RenameRetry { attempts: 0, first_delay_ms: 0 };
    options.atomic_write_custom(fs, retry, path, b"private payload")
}
```

Use `atomic_write_with_custom` for streaming replacements, or `write_new_custom` and
`write_new_with_custom` for exclusive creation. The free `atomic_write_custom` and
`write_new_custom` functions select the default document policy.

## Guarantees & Doctrines

- **Zero App Code**: Zero dependencies on application crates.
- **Never Crash**: `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`.
- **No Unsafe**: the workspace lints forbid `unsafe` code.
