# craft-ipc

`craft-ipc` is how a Craft app talks to another Craft app on the same machine. It has a server, a client, instance discovery, launch on demand and focus hand-off. It has no async runtime. The wire protocol is in [PROTOCOL.md](PROTOCOL.md). The crate depends on no app and on no other Craft crate.

**Native only.** `craft-ipc` does not build for `wasm32-unknown-unknown`, because `interprocess` (local sockets and named pipes) has no wasm32 build. An app gates the dependency by target. The pure data modules (`moniker`, `record`, `payload`, `asset`, `ids`) use only `std` and `serde`.

## Boundaries

- Never crashes: no panics, no `unsafe`.
- Every connection must present a 64 hex token within 2 seconds. The server also checks the peer user id (Unix) or the token user (Windows). At most 16 connections are open, and at most 4 of them are unauthenticated.
- One UTF-8 JSON object per line, at most 4 MiB per line. A request can arrive in pieces with pauses between them. `wire::LineReader` keeps the partial line. Do not read control lines with a stateless `read_line` on a stream that has a receive timeout.
- On Unix, the socket file is mode `0600` inside a `0700` directory. The Windows code paths (named pipe, token user check) have not been run on Windows.

## Modules

| Module | Role |
|---|---|
| `server`, `client`, `wire`, `bulk`, `auth`, `launch`, `activate`, `discovery` | transport, session, frames, focus hand-off, instance records |
| `moniker`, `record`, `payload`, `asset`, `ids` | pure data types |
| `refresh` | `Tracker` (poll, epoch bump, relink) and `write_atomic` for linked files |
| `fetch` | `FrameFetcher`: ticket, a few `BulkClient` workers, a RAM cache keyed `FrameKey` with a byte budget and LRU eviction. `fetch` never waits past its `wait` argument. A late frame still lands in the cache |
| `watch` | `Watcher` thread that polls `Tracker`s: shallow every second, deep every 30 seconds. `IntervalTicker` runs on real time and `ManualTicker` is for tests |
| `trust`, `cache` | `check_executable` guards every launch (absolute path, regular file, executable, owner and mode checks). `private_cache_dir`, `write_private` and `read_private` make a `0700` cache with no `/tmp` fallback |

- `server::Limits` holds the connection limits: auth deadline 2 s, started line 10 s, idle 1 h for unsubscribed control connections, 4 unauthenticated of 16.
- `payload::check_peer_path` is the one path filter: absolute, no NUL, no `..`. The media probe only touches regular files and never opens a FIFO.
- `Server::start_with(config, Extensions { assets, caps })` adds an asset provider and extra caps. `InstanceRecord` keeps `caps` in `extra`. Use `caps()` and `add_cap()`.
- Discovery: each app writes one JSON record, mode `0600`, into a per-user runtime directory. A lock file held for the life of the app shows whether the owner is alive.
- Launch: `launch::resolve` finds an app binary (environment, a read-only parse of the launcher's install record, `PATH`, usual install locations). `launch` starts it only when `trust::check_executable` accepts the file.
- Loopback TCP is available only when the app opts in. It binds 127.0.0.1 and runs the same handshake. A TCP client reads the port and token from the instance record (`Client::connect_tcp`).

## Schemas and fixtures

`schemas/instance-record.schema.json` and `schemas/link-record.schema.json` describe the stored records. `tests/schemas.rs` checks that each schema and its Rust type agree, and that the stored records in `tests/fixtures/` stay valid.

## Tests

A forked child holds a copy of every open file descriptor until exec, so a `flock` released by `drop` can look held. Unit tests that start a process take `crate::SPAWN_GUARD.write()`. Unit tests that publish or scan a discovery lock take `.read()`.
