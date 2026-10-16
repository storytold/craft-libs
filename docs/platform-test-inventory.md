# Platform verification plan

The [JSON inventory](platform-test-inventory.json) is a list of verification needs, not a
record of available hardware or completed tests. Every environment starts **unverified**.
There are no inferred tester names, OS versions or successful runs.

Select an environment that can exercise the changed behavior. In particular, a generic Mac
runner does not establish Touch Bar behavior. The motivation for that check is recorded in
[the quit implementation](../crates/quit/src/lib.rs) and
[issue #3](https://github.com/storytold/craft-libs/issues/3); this inventory does not independently
confirm a reproduction or fix on any model.

For each run, record the candidate SHA/patch, crate/features, exact command or GUI steps,
OS/architecture, relevant hardware/filesystem, observed result and evidence link. Distinguish
mock tests, compilation, native API tests and interactive hardware observations. An unavailable
environment remains unverified; successful compilation cannot fill that gap.

Support is crate-specific. UI WASM checks and settings document-helper checks do not imply
browser filesystem persistence or native printing. A VM print queue test does not establish
physical printer output. File-sync ordering does not establish power-loss survival. Read the
workflows at the candidate revision for automated coverage and the release notes for its scope.
