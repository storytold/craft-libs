# craft-command

`craft-command` is the command layer a Craft app can share: a registry of commands, hooks around each call, a feed of document changes and a JSON Schema per command. Each app keeps its own command table, its `execute` path and its document type. The crate depends on `serde` and `serde_json` only, on no other Craft crate and on no app, and it builds for `wasm32`.

## Boundaries

- Never crashes: no panics, no `unsafe`. A poisoned lock is recovered, not unwrapped.
- The app owns the command table (`CommandSpec` rows), the session type and the error type. The crate is generic over both.
- Plugin commands are added and removed while the app runs. The crate has no plugin loader.

## Modules

- `registry`: `Registry` is built from a static table. Commands can also come and go at run time (`register`, `unregister_owner`). Each dynamic command has an `Owner`. `resolve` returns a `CommandEntry`, either a `Static` row or a dynamic command. `schema` returns the JSON Schema of one command.
- `hooks`: `Hooks` and the `Interceptor` trait. `before` can veto or rewrite a call. `after` sees the outcome. A gate runs only for command ids that match its `Interest` globs. A gate that runs over 2 ms in one call, or that takes more than 6 ms of one 16 ms window, is disabled and the loss is announced. Budgets are kept per owner, so one owner cannot spend another owner's time. A gate whose `security()` returns true is never timed out, and a panic in it vetoes the call.
- `change` and `tracked`: `ChangeBus` delivers `Change` values to subscribers. `Tracked<T>` wraps a value. A write that skips the revision bump is reported as one `Untracked` change when `flush` runs. Undo, redo and background jobs are seen once, like any other change.
- `observe`: `ObserverQueue`, a queue of call events for each subscriber. It holds 1024 events. When it is full, the oldest event is dropped and a lost count grows.
- `schema`: `from_doc` derives a JSON Schema from an existing params doc string, or reports the schema as opaque.
- `json_diff`: `diff` and `patch` for JSON values.
- `conformance` (feature `testing`): the `CommandHost` trait and `run_all`. Each app implements the trait and runs the same checks.

## The call order

An app's `execute` path calls the hooks in this order:

1. Resolve the id in the registry. An unknown id returns the app's existing error.
2. Call `Hooks::before`. A veto returns `CommandError::Vetoed`, and `after` still runs.
3. Run the app's pre-steps, then its enabled check.
4. Run the command.
5. Run the app's tail (journal, history, snapshot).
6. Call `Hooks::after`.
7. Call `Hooks::flush` with the document's `Tracked` value.

`benches/execute.rs` is a criterion benchmark of this path with and without hooks. Run it with `cargo bench -p craft-command`.

## Features

- `testing`: the `conformance` module. Off by default.
