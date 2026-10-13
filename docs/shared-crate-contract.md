# Shared crate contracts

This guide sets expectations for new shared APIs and records the boundaries of existing
services. Read the API and README at the exact dependency revision. Proposals, implementation,
tests and native observations are different kinds of evidence.

## Workspace rules

Crates must serve more than one app and must not depend on application crates. Inherit
workspace metadata and lints, including the unsafe-code prohibition. Retain each library's
never-crash Clippy deny header. The workspace uses Rust 2024, resolver 3 and Rust 1.95 as its
minimum compiler. Code is dual-licensed MIT OR Apache-2.0; retain third-party notices.

## Errors and state

Define invalid input, supported operations, errors and post-failure state alongside each
public API. Use `Result` or `Option` when callers need to react. Use typed errors where callers
need to distinguish cases; do not change an existing error type merely to standardize style.
A fallback or saturating operation needs documented semantics and boundary tests. A lint
header alone cannot prove freedom from runtime panics or resource exhaustion.

State exactly which mutations commit before an error and which cleanup is best effort.
Do not promise rollback across unrelated files, concurrent writers or external side effects.
Bound input-derived allocation and iteration, and test the relevant malformed inputs.

## Atomic persistence

`atomic_write*` stages beside the resolved destination as `.{stem}.{pid}-{counter}.craft_tmp`,
using exclusive creation with collision retries. The suffix is not cryptographically random.
It follows destination symlink chains with a hop limit; it does not perform canonical mount
verification. The parent directory must already exist.

The replacement path writes and syncs the stage, then renames it over the destination.
Staging or rename failure leaves the prior destination bytes unchanged by this operation;
cleanup is attempted, but removal failures and abrupt process termination can leave stages.
There is no copy fallback for a failed replacement rename, including `EXDEV`. `write_new*`
is a separate exclusive-creation API: it tries a hard link, then exclusive copy when linking
is unavailable. The copy fallback can expose a partially written new file; it does not
provide replacement-style atomic visibility.

File sync uses `sync_all`; Apple platforms fall back to plain `fsync` for the specific
unsupported-full-sync errors. Parent-directory sync after publication is best effort on
Unix and a no-op elsewhere. A directory-sync failure does not undo publication or turn it
into a failed save. This ordering is not proof of physical power-loss survival. Windows
rename retry handles transient locks with bounded backoff.

Existing Unix destination mode bits are restored after the writer callback. Replacement
creates a new inode; ownership, extended attributes, ACLs and hard-link identity are not
preserved. New-file permissions are version-specific: consult the selected revision's
[atomic API](../crates/atomic/src/lib.rs). The pending
[permission policy change](https://github.com/storytold/craft-libs/pull/43) introduces normal
creation permissions for documents/exports (`0666` subject to umask/default ACL), explicit
private creation for settings (`0600`), and private replacement staging. Do not assume that
policy is available before the change lands, or use a generic default as a privacy guarantee.

## Settings

The [settings API and README](../crates/settings/README.md) describe section defaults,
comment/unknown-key preservation, schema migration, backup and read-only recovery. A newer
schema or failed migration keeps the store read-only. Invalid TOML is moved aside before
using defaults; if preservation fails, the store remains read-only. Loading does not
automatically restore `.bak`.

Before saving, the store rechecks disk contents. Blank or corrupt input must not replace a
known-good backup. Known values from the store overwrite known keys; comments and unknown
keys are retained where documented. Saves are best effort, last writer wins, with no
multi-process locking. Apps own their schema, scheduling, error reporting and final flush;
a successful atomic write is not an application-level transaction across several files.
Persistent storage is native-only; WASM exposes the trait, document helpers and error types.

## Application shutdown

The [quit API](../crates/quit/src/lib.rs) bridges egui close requests and the optional
macOS app-menu Quit action to deferred `-[NSApplication terminate:]`. Apps run their
unsaved-change decision in `eframe::App::logic` before `on_close_request`, using `CancelClose`
to keep the app open. They request Close only after a save succeeds or discard is confirmed;
asynchronous saves must finish first. Keep prompts accessible when the window is hidden.

The crate does not install `applicationShouldTerminate:`, intercept Dock Quit or POSIX
signals, implement Windows `WM_CLOSE`, or drain arbitrary application buffers. On platforms
without its native quit path, `quit()` returns false and normal window closing continues.
The optional menu replaces the macOS application menu; apps with their own menu route the
Quit item through `on_menu_event`. Menu installation failure leaves the default menu, whose
Quit can bypass the app's prompt. Apps must handle that failure.

The documented eframe/winit integration runs `on_exit` through `applicationWillTerminate:`;
code after `run_native` is not a substitute. Test the actual app's save/close behavior and
relevant hardware separately; this adapter alone does not establish crash-free termination.

## Compatibility and adoption

The [release policy proposal](https://github.com/storytold/craft-libs/pull/41) defines
coordinated, immutable workspace tags and independent consumer upgrades. It is pending until
merged; use the versioning document included in the release you adopt. The intended 0.1.x
line permits compatible fixes and additions; changes requiring consumer action belong in
0.2.0. A release does not require every application to update its pin.

Validate direct internal dependents, then affected application workflows using exact baseline
and candidate revisions. Document API removals and migration paths in the breaking release;
deprecate first when useful. Consumers on immutable older pins keep their old API. Do not make
API removal conditional on upgrading every application. Use the same craft-libs revision for
all its crates within one consumer dependency graph to avoid duplicate type identities.

Support is crate- and feature-specific. Browser UI compilation does not establish native
settings persistence, printing or shutdown support in a browser. See the
[platform verification plan](platform-test-inventory.md) for evidence requirements.
