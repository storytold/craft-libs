# Code style and lints

Use the checked-in [rustfmt configuration](../rustfmt.toml) (`max_width = 150`,
`use_small_heuristics = "Max"`) and nearby code conventions. Inherit package metadata and
lints from [Cargo.toml](../Cargo.toml); the workspace forbids unsafe code. Keep each library's
Clippy deny header for unwrap, expect, panic, unimplemented, todo and unreachable.
The [Clippy configuration](../clippy.toml) allows unwrap, expect, panic and indexing in tests;
production code still needs explicit handling of invalid input and errors.

Use meaningful regression assertions and independently derived expectations. When handling
poisoned mutexes, decide whether invariants can be restored or an error is needed; blindly
taking the inner value is not always safe. Treat thread joins as fallible. Avoid blocking or
allocation where a caller's measured latency contract prohibits them.

Explain non-obvious invariants and compatibility constraints near the code. Keep API docs
accurate about errors, mutation and platform limits. Follow [type contracts](type-system-contracts.md)
and [verification guidance](ci-and-verification.md).

Use original or compatibly licensed code and preserve attribution and notices. See
[CONTRIBUTING.md](../CONTRIBUTING.md) for asset and crate requirements. Do not copy another
person's commit identity; use your own authorized identity and chosen credit. Write concise
commit subjects and explain important tradeoffs in the body. PRs should state the problem,
resulting behavior and actual verification. Keep changes focused; the repository's selected
merge method determines integration, with no blanket squash or history-rewrite requirement.
