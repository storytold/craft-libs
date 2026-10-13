# Types, errors and state

Use explicit `Result` or `Option` for failures callers need to handle. Prefer typed errors
when callers must distinguish causes; `thiserror` is an available design choice, not a
workspace mandate. Existing APIs include `io::Result`, boxed errors and
[`Result<T, String>` printing helpers](../crates/ui/src/printing.rs). Changing a public error
type is an API compatibility decision, not a lint cleanup.

Document invalid input, unsupported operations, mutation ordering and post-failure state.
An empty success value must not conceal an error. Fallbacks and saturation need explicit
semantics; they are not substitutes for checked arithmetic when losing information is unsafe.
Do not add `todo!`, `unimplemented!`, unwrap or panic paths to shipped code.

Validate input-derived indices and UTF-8 boundaries; use `.get()` for fallible access.
Check lengths, offsets, division and numeric conversions. Bound allocations and recursion
from untrusted data. Reject non-finite values when the operation requires finite inputs.
Choose limits and tests based on actual callers and workloads.

Keep logical identities stable across label changes, localization and reordering. Preserve
app-owned state and intentional product differences when extracting a shared mechanism.
For future domain types, establish the contract with real consumers first. This revision
contains no `craft-time` crate and imposes no speculative `Tick` or SMPTE API on contributors.

Use [shared services](shared-services-architecture.md) for persistence and quit boundaries,
and the [initial release policy](https://github.com/storytold/craft-libs/pull/41) for
compatibility rules; use the versioning document included in the selected release.
