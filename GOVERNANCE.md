# Governance

The `craft-libs` workspace is intentionally conservative. The repository grows by empirical evidence, not by accumulation.

## Change classes

**Editorial correction** restores an existing rule, documentation typo, or comment, and may land as a patch.

**Repository capability** adds tooling, compiler checks, test fixtures, or optional features without changing the core shared contract.

**Normative change** modifies authority, architectural invariants, work authorization, evidence semantics, autonomy boundaries, or shared crate API contracts. It requires a dedicated issue and an explicit cross-application compatibility assessment (`cargo xtask compat`).

## Anti-bloat test

Before adding a new shared crate or abstraction, determine whether the need can be satisfied by:
- existing primitives in `craft-atomic`, `craft-settings`, `craft-quit`, or `craft-ui`;
- application-local code inside the consuming Craft application;
- reference, composition, or qualification.

If the functionality is not needed by multiple applications, it does not belong in `craft-libs`.

## Evidence requirement

Every proposed change must state the problem it closes and provide empirical evidence that the problem exists. New abstractions without an observed consumer failure mode remain proposals. Every crash or boundary fix must include a synthetic regression test proving failure prior to the fix.

## Clean-room & proprietary independence policy

Contributions to `craft-libs` must maintain strict independence from third-party proprietary commercial products. Pull requests, issues, discussions, commits, comments, and code must not reference proprietary commercial products outside the ArtCraft and Storytold ecosystem. Features and workflows must be defined strictly in terms of domain standards, public open specifications (ISO, ICC, SMPTE, Unicode, W3C), or ArtCraft/Storytold application identities. Submissions referencing external proprietary software will be rejected or sanitized at triage.

## Human authority

Repository maintainers retain release, tagging, and normative-change authority. Autonomous automation may prepare, verify, and recommend; it does not manufacture approval.
