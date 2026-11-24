# Shared crate behaviour

Use the documentation at the dependency revision the app pins. The crate API and tests define
what callers can rely on. When documenting a change, describe which errors callers can handle,
what state remains after failure, and which platforms and features were exercised.

## Atomic file replacement

[`craft-atomic`](../crates/atomic/README.md) writes and syncs a staging file next to the
destination before renaming it into place. If staging or the replacement rename fails, the
operation leaves the previous destination bytes intact. A failed cleanup or abrupt process
exit can leave a staging file. Parent directory sync is best effort after publication.
Tests can verify the order of those operations. Durability after power loss depends on the
filesystem and device.

Use `write_new` for exclusive creation. If hard linking is unavailable, its copy fallback can
make the new file visible while it is being written. Choose private creation permissions
explicitly for files that need them and check the platform guarantees at the selected revision.

## Settings and recovery

[`craft-settings`](../crates/settings/README.md) opens settings with a newer schema or a failed
migration in read-only mode. Invalid TOML is moved aside before defaults are used. If the file
cannot be preserved, the store stays read-only. Consumers can use the load report to explain
the state to the user.

The store rechecks the file before saving and preserves comments and unknown keys when it can
merge valid on-disk changes. The settings store uses last-writer-wins semantics. Applications
that need coordination across processes or across multiple files supply it themselves.

## Platform source guards

[`craft-testkit`](../crates/testkit/README.md) supplies source guards for known platform
regressions. They report matching source locations. Keep runtime verification at the
application boundary for behaviours such as whether Quit prompts before closing with unsaved
changes, or whether a shader compiles on the browsers and devices the app supports.
