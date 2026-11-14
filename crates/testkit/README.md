# craft-testkit

Source-level platform regression guards and verification utilities for the Craft application suite.

## Overview

`craft-testkit` provides lightweight, non-panicking source guards that prevent applications in the Craft ecosystem from repeating platform defects that were previously diagnosed and resolved in peer applications:

1. **`predefined_quit`**: Flags direct calls to `muda`'s `PredefinedMenuItem::quit` outside comments, strings and character literals, also with whitespace or a line break around `::`, in the qualified form `<muda::PredefinedMenuItem>::quit`, and through a `use muda::PredefinedMenuItem as ...` rename. On macOS, this predefined menu item routes directly to `NSApplication terminate:`, completely bypassing application dirty document checks and unsaved changes save prompts. Applications should instead wire Quit through `craft-quit` lifecycle interceptors.
2. **`wgsl_float_literals` / `wgsl_float_literals_in`**: Scans WGSL shader sources and flags decimal and hexadecimal float literals whose value, rounded to `f64`, exceeds `f32::MAX` (such as `3.40282347e38` or `0x1p+500f`), or 65504, the largest finite `f16`, for a literal with an `h` suffix. A literal that exceeds the maximum by less than `f64` precision rounds to the maximum and is not reported. Integer literals are not checked. Some native compilers round such literals down, but browser WebGPU compilers reject them with pipeline compilation errors, forcing applications into CPU fallback rendering. `f32::MAX` can be written as the hex float `0x1.fffffep+127f`, which equals it, or as the decimal `3.4028234663852886e38`, which rounds to it.
3. **`std_instant`**: Flags `std::time::Instant` outside comments and strings: the full path, `<m>::Instant` where a `use` binds `m` to `std::time`, and each `use` that imports `std::time::Instant` or `std::time::*`, under any name. The parts of a path may be split by whitespace, comments or line breaks. Other crates' `Instant` (such as `web_time::Instant` or `tokio::time::Instant`) is not flagged, and neither is an item whose `cfg` attribute is false on `wasm32-unknown-unknown`; code before that attribute or after its item on the same line is still checked. A `mod name;` inside such an item also skips the module's file (`name.rs` or `name/mod.rs`, or the file a `#[path]` attribute names) and the files of the modules declared in it, when they are among the files passed in and no declaration outside such items also reaches them. On that target, `std::time::Instant::now()` panics at runtime with `"time not implemented on this platform"`. Code targeting the web must use `web_time::Instant`.
4. **`rust_sources`**: Recursively discovers a crate's `.rs` source files while excluding integration tests (`tests/`), test modules (`tests.rs`, `tests_*.rs`, `*_tests.rs`, `*_test.rs`), build artifacts (`target/`, any folder holding a `CACHEDIR.TAG` file that starts with the line `Signature: 8a477f597d28d172789f06886806bc55`, `.git/` and other folders whose name starts with `.`) and symbolic links, which it does not follow. A file over 8 MiB, more than 256 MiB in all or more than 50,000 files returns an error; `rust_sources_with_limits` takes other limits. It is not built for `wasm32`.

## MCP test client: `mcp`

The `mcp` feature adds `craft_testkit::mcp` and the `serde_json` dependency. The module is built on every target except `wasm32`, and it opens no network connection.

`McpChild::spawn(path, args, env)` starts an MCP server program with its stdin and stdout piped. The variables in `env` are set for the child only; the test process's environment is not changed. `McpChild::connect(reader, writer, limits)` talks to a server that runs in the test process. Each message is one line of JSON-RPC 2.0, as the [MCP stdio transport](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports) describes.

| Call | What it does |
|---|---|
| `initialize(version)` | Sends `initialize` with `version` (`mcp::PROTOCOL_VERSION` is `2025-06-18`), waits for the reply, then sends `notifications/initialized`. A reply with another version returns `UnsupportedVersion` and closes the connection. |
| `request(method, params)`, `request_within(.., timeout)` | Sends a request with a new numeric id and returns the `result` of the reply with that id. Replies with other ids are skipped. An error reply returns `Rpc` with its code, message and data, and the connection stays usable. |
| `call_tool(name, args)`, `call_tool_within(.., timeout)` | Sends `tools/call` and returns the tool result unchanged, `content` and `isError` included. |
| `list_tools()` | Reads `tools/list` pages through `nextCursor` and returns the tools as sent. More than `max_pages` pages returns `TooManyPages`, and tools that add up to more than `max_line_bytes` of JSON return `ToolListTooLarge`. |
| `notify(method, params)` | Sends a notification, which has no id and gets no reply. |
| `take_notifications()` | Returns the notifications kept so far without waiting. |
| `close()` | Closes the child's stdin, waits for it to exit, stops it after the grace period, and returns its exit status, whether it was stopped, and the stderr kept. |

While a call waits, the client answers the child's `ping` with an empty result and its other requests with error -32601. A line on stdout that is not a JSON-RPC 2.0 message, a reply with neither or both of `result` and `error`, a line over the length limit, a missed deadline and the end of stdout each return an error, close the connection and stop the child; later calls return `Closed`. Dropping an `McpChild` stops and reaps a child that is still running. Setup returns a `Result` and nothing in the module panics.

`McpLimits` holds the limits. These defaults are the crate's own values:

| Field | Default | Range |
|---|---|---|
| `max_line_bytes` | 64 MiB, for one stdout line and for the whole `list_tools` result | at least 1 |
| `request_timeout` | 30 s | more than zero, at most 1 hour |
| `initialize_timeout` | 60 s | more than zero, at most 1 hour |
| `close_grace` | 2 s | at most 1 hour |
| `max_pages` | 64 | at least 1 |
| `max_notifications` | 0 (notifications are not kept); the oldest is dropped when the queue is full | at most 4096 |
| `stderr_tail_bytes` | 0 (stderr goes to the null device); otherwise the last bytes written are kept | at most 16 MiB |

`McpLimits::timeouts_from_var(name)` sets both deadlines from a variable that holds a whole number of seconds from 1 to 3600; `mcp::TIMEOUT_VAR` (`CRAFT_TEST_MCP_TIMEOUT_SECS`) is the default name. Any other value returns an error.

```rust
use craft_testkit::mcp::{McpChild, PROTOCOL_VERSION};

let mut mcp = McpChild::spawn(env!("CARGO_BIN_EXE_my-cli"), ["mcp"], [("MYCRAFT_CONFIG_DIR", dir.path())])?;
mcp.initialize(PROTOCOL_VERSION)?;
let result = mcp.call_tool("doc.new", serde_json::json!({}))?;
assert_eq!(result["isError"], false);
assert!(mcp.close()?.status.is_some_and(|s| s.success()));
```

The tests that start a real child are in `tests/mcp_child.rs`, a target with `harness = false`: the child is that test program, and the standard harness writes its own lines on stdout. Both test targets build to nothing on `wasm32`.

### Origin

| Module | Source repository | Path | Revision |
|---|---|---|---|
| `mcp` | storytold/craft-ai-integrations | `crates/ai-mcp/src/apps/child.rs` (`Conn`, `request`, `call_tool`, `MAX_LINE`, deadlines) and the 14 tests of `child_tests.rs`, written again here | `e8b31cb0` |
| `mcp` | storytold/vectorcraft | `apps/vectorcraft-cli/tests/it/mcp_e2e.rs` | `460d2119` |
| `mcp` | storytold/photocraft | `apps/photocraft-cli/tests/it/cli.rs` (`mcp_recv`) | `58d11d91` |

### Migration

| Consumer | Helper | State |
|---|---|---|
| VectorCraft | `apps/vectorcraft-cli/tests/it/mcp_e2e.rs` | Not switched |
| PhotoCraft | `apps/photocraft-cli/tests/it/cli.rs` | Not switched |
| CADCraft, DeckCraft, DesignCraft, EffectCraft | the MCP helpers in their CLI tests | Not switched |
| FigCraft, FilmCraft, GridCraft, LightCraft, PdfCraft, SoundCraft | the MCP helpers in their CLI tests | Not switched |
| craft-ai-integrations | `crates/ai-mcp/src/apps/child.rs`, production code | Kept as it is |

## Epistemic Scope: Syntactic Guards vs Runtime Validation

Source-level guards provide deterministic syntactic evidence about codebase structure:

### What They Establish
- Checked source files do not contain forbidden symbol invocations, known out-of-range literal representations, or platform-hazardous imports.
- Reusable, deterministic checks that run in milliseconds as regular Cargo tests without external runners.

### What They Cannot Establish
- **Runtime execution safety**: Syntactic checks do not verify whether code branches actually execute at runtime.
- **Semantic type resolution**: The scanners follow `use` renames within one file only. They do not resolve paths through other modules or re-exports, a rename of `std` itself, or expand macros.
- **Complex target predicates**: `std_instant` evaluates `cfg` predicates (`all`, `any`, `not`, `unix`, `windows` and the `target_*` keys) for `wasm32-unknown-unknown`, on outer attributes for the item that follows and on inner attributes (`#![cfg(...)]`) for the rest of the module, and on `mod name;` declarations for the module's file and its descendants. It compares file paths as text, so the files must come from one `rust_sources` call or be named the same way. A predicate that depends on a feature or on `test` is not known from the source, so its item is checked. Applications should scope `std_instant` tests to the specific crates that compile into their web distribution.
- **Driver and host OS behavior**: Syntactic checks do not verify WebGPU driver compatibility or host window manager event dispatch.

Source guards serve as fast, zero-dependency regression gates in application test suites, complementing platform integration tests and CI workflows.

## Consumer Adoption Examples

### 1. PhotoCraft: Adopt `wgsl_float_literals` and `predefined_quit`

In `photocraft/crates/gpu/src/lib.rs`, replace the in-tree `decimal_float_literals` helper with `craft_testkit::wgsl_float_literals`:

```rust
#[test]
fn shader_float_literals_fit_f32() {
    let bad = craft_testkit::wgsl_float_literals(SHADER);
    assert!(
        bad.is_empty(),
        "float literals outside f32 finite range reject on WebGPU (Tint):\n{}",
        bad.iter().map(|f| f.to_string()).collect::<Vec<_>>().join("\n")
    );
}
```

In `photocraft/apps/photocraft/tests/native_menu_tests.rs`:

```rust
#[test]
fn no_unintercepted_predefined_quit_in_menu() {
    let sources = craft_testkit::rust_sources("src").expect("scan sources");
    let findings = craft_testkit::predefined_quit(&sources);
    assert!(
        findings.is_empty(),
        "predefined muda Quit bypasses dirty document save prompts:\n{}",
        findings.iter().map(|f| f.to_string()).collect::<Vec<_>>().join("\n")
    );
}
```

### 2. FilmCraft: Adopt `std_instant` and `wgsl_float_literals`

In `filmcraft/crates/engine/src/wasm_clock_tests.rs`, replace in-tree textual search with `craft_testkit::std_instant`:

```rust
#[test]
fn no_std_instant_in_code_the_web_build_runs() {
    let sources = craft_testkit::rust_sources(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
        .expect("scan engine sources");
    let findings = craft_testkit::std_instant(&sources);
    assert!(
        findings.is_empty(),
        "std::time::Instant panics on wasm32; use web_time::Instant:\n{}",
        findings.iter().map(|f| f.to_string()).collect::<Vec<_>>().join("\n")
    );
}
```

## Origin

| Module | Source | Path | Commit |
|---|---|---|---|
| `guards` | storytold/craft-libs#163 (zachshallbetter), with the rules of PhotoCraft's shader float test and FilmCraft's web clock test | `crates/testkit/src/guards.rs` | `e92825e427` |

## Migration

| Consumer | Guard | Replaces | State |
|---|---|---|---|
| PhotoCraft | `wgsl_float_literals` | `crates/gpu/src/tests.rs` scanner | Not started |
| FilmCraft | `std_instant` | `crates/engine/src/wasm_clock_tests.rs` search | Not started |
