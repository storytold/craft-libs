# Craft AI model

`craft-ai-core` is the data model and contract for AI in the Craft Apps: connections, models,
capabilities, roles (tools), requests, jobs, consent, request history, the shared store's file
format, and the declarative route spec. It has no I/O, no async runtime and compiles for
`wasm32-unknown-unknown`. It depends only on `serde` and `serde_json`.

One connection is set up once (in the launcher or any app) and every app that has the tool uses it.
Each app keeps its own interface for each tool.

## Boundaries

- Never crashes: the crate denies `unwrap`, `expect`, `panic` and friends, and the workspace
  forbids `unsafe`.
- No secrets: a connection holds a reference to a secret (`SecretRef`), never the value.
  `SecretString` does not print.
- No app code: an app is only an `AppId` string. The contract is described in
  [docs/ai-shared-layer.md](../../docs/ai-shared-layer.md) and the declarative route spec in
  [docs/ai-declarative-adapter.md](../../docs/ai-declarative-adapter.md).

Later pull requests add the desktop half (secret stores and the shared store file), the client,
the provider adapters and egui widgets as separate crates.

## Platforms

All platforms, including `wasm32-unknown-unknown` (checked in CI). No I/O.

## Usage

Depend on it for the types. Worked examples: `tests/custom_tools.rs`, `tests/mock_conformance.rs`, `tests/spec_calls.rs`; the contract is in [docs/ai-shared-layer.md](../../docs/ai-shared-layer.md).
