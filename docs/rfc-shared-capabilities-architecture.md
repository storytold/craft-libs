# RFC: extracting shared capabilities

Status: proposed contributor workflow. This document does not announce a new crate,
compatibility runner, consumer adoption or platform qualification.

`craft-libs` holds reusable mechanisms. Applications keep document state, commands, undo,
product policy, localization and their own UI choices. Existing crates are listed in the
[README catalog](../README.md#libraries); open PRs are not available dependencies.

## Extraction protocol

1. **Find two actual consumers.** Inspect their implementations and callers at recorded
   revisions. Identify the shared behavior, state owners, workloads and platform assumptions.
   A list of possible apps is not adoption evidence.
2. **Compare behavior.** Use representative fixtures and boundary cases from both consumers.
   Classify differences as defects, intentional app policy or unresolved assumptions. Keep
   intentional differences explicit in adapters or APIs; retain unresolved behavior in the app.
3. **Write the contract.** Define errors, post-failure state, resource limits, performance
   expectations and platform boundaries. Follow the [shared contracts](shared-crate-contract.md).
   Do not hide unsupported operations behind successful empty results.
4. **Verify the change.** Run the repository's relevant checks and meaningful comparisons in
   isolated consumer worktrees. Record the baseline pin, candidate commit/patch, consumer SHA,
   command, executed test count and result. Keep working trees and real user data untouched.
   Separate pre-existing failures from candidate regressions; zero comparisons is unverified.
   Exercise affected native workflows on a suitable OS/device and retain evidence.

There is no required `cargo xtask compat`, `compat-check` or `ci` command in this revision.
Use commands present in the checked-out repositories. A future runner may automate this
protocol, but its absence does not prevent a focused, manually recorded comparison.

## Evidence and release decisions

Label claims as proposed/documented, implemented, tested or observed on a named environment.
A passing mock test does not qualify native behavior, and a GUI smoke test does not prove
power-loss durability. Include source revision, actual environment, test scope, results and
limitations. Keep useful failed comparisons and rejected assumptions with the change.

The [consumer map](consumer-dependency-map.md) and
[platform plan](platform-test-inventory.md) are starting points for collecting evidence;
they do not certify adoption, hardware access or passing tests. Scope compatibility checks
to affected consumers and workflows rather than requiring a suite-wide upgrade.

The [initial release policy](https://github.com/storytold/craft-libs/pull/41) defines
coordinated, immutable workspace tags and independent consumer upgrades. Use the versioning
document included in the release you adopt. Its 0.1.x
line permits compatible fixes and additions; changes requiring consumer action belong in
0.2.0. A release does not require every application to update its pin.

Measure hot-path changes using representative workloads and input sizes. Shared extraction
should preserve responsiveness and memory behavior as well as results. Use fuzz/property
coverage where it meaningfully exercises parsers or invariants; do not require an unrelated
tool or ceremony for every small change.
