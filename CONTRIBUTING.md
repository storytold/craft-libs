# Contributing to craft-libs

Start with the [library catalog](README.md#libraries) and the affected crate's README and API.
Shared crates provide mechanisms useful to multiple apps; document state, commands, undo,
localization and product policy stay with the application.

## Propose and implement a focused change

For a new extraction, inspect two actual consumers at known revisions. Compare their callers,
behavior and workloads. Distinguish defects from intentional app differences and retain
unresolved policy in the app. Describe shared errors, post-failure state and platform limits
before choosing the API. Existing public contracts must remain compatible within their release
line; do not redesign them only to make every crate look alike.

Code uses Rust 2024, resolver 3 and a minimum compiler of Rust 1.95. Workspace lints forbid
unsafe code; library roots deny Clippy's unwrap, expect, panic, unimplemented, todo and
unreachable lints. Check input-derived indices, arithmetic, allocations and recursion bounds.
Return useful errors and preserve state on rejection. See [type contracts](docs/type-system-contracts.md)
and [code style](docs/code-style-and-lints.md).

Code is MIT OR Apache-2.0. Do not copy proprietary implementation or incompatible licensed
code. Record bundled asset sources, authors and licenses in [ATTRIBUTION.md](ATTRIBUTION.md)
and preserve required notices. Keep large corpora outside the repository; prefer small
synthetic regression cases. Check the actual crate's asset/build setup instead of assuming
an external corpus or font-fetching tool exists.

## Add a crate

Create `crates/<name>` with package name `craft-<name>`. Inherit `version`, `edition`,
`license`, `repository`, `rust-version` and workspace lints in its manifest. The workspace's
`crates/*` member glob includes it; add its path to `[workspace.dependencies]`. Add a README
and a catalog entry explaining scope, features, platforms and usage. A crate must not depend
on any application. The catalog at the selected revision defines what is available; an open
PR for a media-time crate or compatibility runner is not a shipped dependency.

## Validate and submit

Add regression tests that fail for the defect or independently check the new behavior.
Exercise meaningful boundaries and app-specific differences. Follow
[CI and verification](docs/ci-and-verification.md) for available workspace and targeted WASM
commands. For an extraction or behavior change, compare affected consumers in isolated
worktrees at exact baseline and candidate revisions. Check package names before choosing
commands, record nonzero test counts, and separate baseline failures from new regressions.

Native service changes also need suitable OS/workflow evidence. A unit test or cross-compile
cannot establish interactive save/close behavior, physical printing or power-loss durability.
Report unavailable checks explicitly. For documentation-only changes, validate local links,
command availability and claims against source; do not present an older build as new evidence.
Required merge/release checks still apply to the final combined source.

Use your own authorized commit identity and preferred contributor credit. Never copy another
contributor's name/email into Git configuration. Give the PR a concise problem/change summary,
verification at the reviewed revision and remaining limitations. Link relevant issues without
claiming they are fixed until their acceptance criteria are met. Keep commits reviewable;
maintainers choose the merge method. No repository-wide squash requirement is established here.

## Releases and consumer upgrades

The [initial release policy](https://github.com/storytold/craft-libs/pull/41) defines
coordinated, immutable workspace tags and independent consumer upgrades. Use the versioning
document included in the release you adopt. Its 0.1.x
line permits compatible fixes and additions; changes requiring consumer action belong in
0.2.0. A release does not require every application to update its pin.

Keep every craft-libs dependency in a consumer graph on one revision. Test an upgrade in that
application before changing its shipped pin. Proposed future APIs and support promises do not
apply retroactively to existing pins. [Shared services](docs/shared-services-architecture.md)
describes the boundaries to check when adopting persistence, shutdown, printing and UI helpers.
