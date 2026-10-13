# CI and verification

Run these workspace gates for code changes and before merging the combined source:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

The workspace uses Rust 1.95 or newer. Follow host-specific allocation/build instructions
when running these commands. For docs-only edits, validate links, commands and source claims;
state that scope rather than quoting old test counts as validation of the new revision.

## Targeted platform checks

With the WASM target installed, the supported check surface is:

```sh
cargo check -p craft-ui --all-features --locked --target wasm32-unknown-unknown
cargo check -p craft-settings --locked --target wasm32-unknown-unknown
```

The settings check covers its trait, document helpers and errors, not persistent storage.
Native printing and shutdown have separate platform contracts. Do not advertise all workspace
crates or every architecture as browser-compatible from these commands.

Inspect [Shared UI](../.github/workflows/ui.yml) and
[Print spooler](../.github/workflows/print.yml) for current job matrices and path filters.
They cover selected native Linux/macOS/Windows jobs and UI WASM; a docs-only PR may not trigger
them. Workspace release validation is introduced by the
[release policy change](https://github.com/storytold/craft-libs/pull/41); inspect the workflows
present at the selected revision. Report actual candidate check results before release,
including all required final-source jobs.

## Consumer comparisons

No `cargo xtask ci`, `cargo xtask compat` or `compat-check` command is required or available
in this revision. Inspect the affected consumer's manifest and instructions and run its real
commands in isolated worktrees. Record its SHA, baseline resolved craft-libs pin, candidate
SHA/patch, features, commands, executed test counts, result and environment. Preserve the
consumer's normal checkout and real user data. Zero tests or zero comparisons is unverified.
Baseline failures must be distinguished from candidate regressions; an unresolved baseline
failure is not evidence that the candidate works.

For changes in native behavior, exercise the app workflow on the relevant OS/device. Mocked
filesystem failures test error handling; compilation checks an API surface; interactive save,
cancel, close and reopen checks app behavior. Physical Touch Bar or printer claims need the
corresponding hardware evidence. None of these alone establishes physical power-loss survival.
Keep limitations and evidence links with the PR so the result can be reviewed at its exact SHA.
