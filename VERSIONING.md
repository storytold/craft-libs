# Versioning Policy

Shared crates in `craft-libs` follow Semantic Versioning (SemVer 2.0.0).

## Release cadence

- **Major (`X.0.0`):** Breaking changes to public crate APIs, shared service contracts, or core operating invariants.
- **Minor (`0.X.0` or `X.Y.0`):** Backward-compatible additions, new widgets in `craft-ui`, new non-breaking capabilities, or new target platforms.
- **Patch (`0.X.Y` or `X.Y.Z`):** Backward-compatible bug fixes, defensive input hardening, and documentation updates.

## Decoupled crate versioning

Crates in the `craft-libs` workspace maintain independent versions and per-crate changelogs. Never infer cross-crate or cross-application compatibility from matching version numbers.

## Cross-application compatibility

Matching version numbers do not imply downstream application compatibility. Downstream consumer applications pin explicit Git revisions or crate versions and validate compatibility via `cargo xtask compat` before advancing pins.
