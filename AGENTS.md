# Agent instructions

Shared Rust libraries for the Craft Apps. Read [CONTRIBUTING.md](CONTRIBUTING.md) and the
affected crate's README/API before changing it. The shared standards are in
[craftrules](https://github.com/storytold/craftrules): `AGENTS.md`, `standards/never-crash.md`,
`standards/engineering.md` and `standards/licensing.md`. If unavailable, report that limitation;
these public instructions and the checked-out source remain usable.

- Keep the never-crash Clippy deny header in each library root and inherit workspace lints,
  including `unsafe_code = "forbid"`. Handle invalid input and errors explicitly.
- New crates go in `crates/<name>`, are named `craft-<name>`, inherit workspace package metadata
  and lints, and are registered in workspace dependencies. Do not depend on app crates.
- Identify two actual consumers before extracting a capability. Compare behavior, preserve
  intentional app differences, and document errors, state ownership and platform boundaries.
- Preserve unrelated changes and consumer pins. Use isolated worktrees for compatibility
  comparisons and record exact source identities. Follow host build/storage rules.
- Use the existing commands in [CI and verification](docs/ci-and-verification.md). The merge
  gates are formatting, workspace Clippy and tests; feature/platform checks follow the changed
  surface. For docs-only edits, check links, commands and source claims and report that scope.
- Distinguish proposals, implementation, executed tests and native observations. Never turn
  an unverified consumer inventory or requested hardware profile into passing evidence.
- Use the contributor's own authorized Git identity. Never configure another person's name or
  email as a repository-wide author. Honor username-only credit preferences and preserve
  existing contributors' credits. Do not infer a public display name or change it.
- Keep commits and PRs focused, with accurate validation and limitations. Follow the selected
  repository merge method; these instructions do not mandate squashing or rewriting history.

Service boundaries are in [shared services](docs/shared-services-architecture.md); API design
and style guidance are in [type contracts](docs/type-system-contracts.md) and
[code style](docs/code-style-and-lints.md). Proposed crates and tooling are not prerequisites.

Read the [shared contracts](docs/shared-crate-contract.md) and
[extraction RFC](docs/rfc-shared-capabilities-architecture.md). The
[consumer inventory](docs/consumer-dependency-map.md) and
[platform plan](docs/platform-test-inventory.md) require evidence before claiming adoption
or completed verification.
