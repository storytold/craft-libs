# Native boundary: an exploratory ownership note

This is **experimental evidence, not an accepted architecture or a published native
component**. No shared native host, Swift kit, engine, or FFI ABI is proposed here.
The [mobile RFC](https://github.com/storytold/craft-contrib/pull/60) and
[craft-mobile PR](https://github.com/storytold/craft-libs/pull/632) remain open.
Its submitted [`CraftApp` contract](https://github.com/storytold/craft-libs/pull/632/files)
extends `eframe::App`, with `open_file`, `action`, `snapshot`, and lifecycle hooks;
its egui/wgpu host runs in native shells. Its author has described
[a later native-chrome experiment](https://github.com/storytold/craft-contrib/pull/60#issuecomment-6919840064)
using Rust panel descriptions as local and unpushed; this note cannot assess that
later implementation. The existing
Rust-only native-app rule would require an explicit approved exception for shared
Swift code; this note grants none. The separate [policy proposal](https://github.com/storytold/craft-contrib/pull/63)
remains open.

An independently examined local PdfCraft iOS proof of concept uses typed generated
Swift/Rust calls, a serialized Rust worker, existing automation and engine state,
and bounded raster previews. The [upstream app proposal](https://github.com/storytold/pdfcraft/pull/7549)
provides a review target; its current source and behavior have not been revalidated
here. The examined POC is not an adopter of craft-mobile
and demonstrates neither a shared host nor cross-app compatibility. Its Swift Files
coordination and save-copy workflow, page/text limits and PDF operations are app
choices, not evidence for a generic UI or document engine. The POC's renderer is
not the egui canvas path. Neither route is selected or excluded here.
This exploration does not change the app's dependency or require production adoption.

| Responsibility | Owner / existing API |
|---|---|
| Engine models, permissions, sessions, commands, undo, capability/tool identity and semantics | Each app's Rust engine and automation; native presentation calls them, never duplicates them |
| Transport, FFI ownership, actor/thread lifetime, cancellation, input and output bounds, cleanup and error presentation | App adapter and OS host; decide and test per transport |
| Files provider access, scoped URLs, coordination, save/export and recovery | App adapter and platform; preserve each app's policy and lifetime |
| Optional JSON-RPC envelopes and bounded input for a message-based adapter | `craft-jsonrpc::{Limits, parse_json, process, Request, RpcError, ParseError}`; not required for typed FFI |
| MCP initialize and tool envelopes if a host actually speaks MCP | `craft-mcp::{Session, Policy, tool_call, tool_list}`; app still provides identity, routes, permissions and tools |
| Picker outcomes and evidence | `craft-platform::dialog_outcome::{Capability, Outcome, DialogError}`; not a native picker implementation |
| Desktop window policy and optional egui integration | `craft-window`; not a mobile renderer |
| Web host messages | `craft-web-host`; a web-specific contract, not an FFI or mobile contract |

[`native_adapter_examples.rs`](../crates/jsonrpc/tests/native_adapter_examples.rs)
exercises two synthetic, independent drawing and spreadsheet routers using the
existing JSON-RPC envelope, including unknown methods and explicit parse/batch
limits. These are not real app integrations and do not test Swift, rendering,
permissions, cancellation or OS lifecycle. They demonstrate an option only if an
app elects JSON-RPC; forcing PdfCraft's typed bridge through JSON-RPC would change
its contract without evidence of benefit. The example records notifications without
dispatching them to either synthetic app; a real host must choose and test its
notification policy.

`parse_json` checks the bytes supplied to it, but a host must bound reads and FFI
allocation *before* constructing that buffer. `process` bounds batch dispatch, not
response size. A production adapter must separately bound output, preserve domain
errors, handle notification semantics, define trust/authentication when needed,
and keep limits appropriate to the app. Do not silently substitute empty success
for malformed or over-limit data. `craft-jsonrpc::control` and
`craft-control-protocol` have different legacy contracts and must not be mistaken
for JSON-RPC or substituted without an explicit compatibility decision.

Before any shared native boundary is extracted, compare real independent adapters,
including the proposed host, for lifecycle, ownership, error propagation, accessibility
and security-scoped files; identify duplicated, app-neutral behavior and test it.
No PdfCraft integration change is requested by this note.
