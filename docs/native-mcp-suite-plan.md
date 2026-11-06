# Native MCP across the Craft suite: proposed plan

**Status:** draft for maintainer review. This PR documents a proposal; it does not add a crate or change an app's MCP behaviour.

## The user problem

A person opens and switches between Craft apps during one agent conversation. Today the MCP process may start first, bind to a new headless document, and stay there after the real app opens. Starting a desktop control port may require a command-line flag and restarting the app. Registering every native server, or exposing every engine command, also makes discovery expensive. The intended experience is: enable agent access once, open an app normally, see that it is ready, and move between open documents and apps without restarting the conversation or editing the wrong document.

These are **two separate costs**: tool/catalogue tokens and connection/startup friction. Moving code into `craft-libs` alone fixes neither.

## Scope and ownership

The proposal covers the native MCP integrations in PhotoCraft, LightCraft, CADCraft, FilmCraft, VectorCraft, PdfCraft, DesignCraft, WordCraft, DeckCraft, GridCraft, EffectCraft, SoundCraft and FigCraft. It does not depend on a third-party gateway. Survey each app's `docs/mcp.md` and MCP/control implementation before migration: tool availability, desktop versus headless mode, auth and command semantics differ.

| Shared infrastructure in `craft-libs` | Remains with each app |
| --- | --- |
| Authenticated control transport, endpoint discovery, deadlines and explicit failure states, as proposed for `craft-control` | Control methods, editor dispatch, UI-thread rules, document model and save/undo behaviour |
| Optional compact MCP discovery helpers, response budgets, pagination and schema envelope | Command IDs, search ranking, parameter definitions and command validation |
| An attach/status contract and conformance fixtures | App startup preference and UI, consent, instance/document selection and confirmations |
| Reusable measurements and contract tests | App-specific availability in GUI/headless mode, batching and filesystem boundaries |

Use the [proposed `craft-control` work](https://github.com/storytold/craft-libs/issues/32) for the shared control channel rather than duplicating its server, client, token or descriptor in an MCP crate. This follows the [shared-library candidate criteria](https://github.com/storytold/craft-libs/issues/2): compare existing implementations, prove a second consumer and preserve app behaviour. The separate `craft-ask-agent` work concerns agent-initiated requests and edit ownership; this proposal should interoperate with it, not replace it. `craft-ai-core` concerns model connections and AI jobs; it is not the MCP transport.

A shared library is **not** one suite-wide MCP endpoint. A single stable connector, if required, is a thin separate integration built on these app contracts, not a registry of all 13 applications' commands inside `craft-libs`.

## Proposed behaviour and contracts

### 1. App-first readiness without unsolicited access

- Each app offers an in-app agent-access control and an explicit, persisted opt-in to start its authenticated control listener on normal launch. Keep existing explicit CLI modes. A fresh owner-protected endpoint descriptor and token belong to that listener lifetime. The descriptor may live in the app's settings folder, but the token must not persist in general preferences or documents. If the control security work is not ready on a platform, do not auto-enable its editing listener there.
- The existing native MCP process may already be running because an agent client launched it. It discovers the app's live endpoint on demand and can attach **after** app launch; a restart of either process is not required. If the app closes or restarts, status changes and the next safe operation reconnects. A broken reply is an unknown outcome and is never silently retried as an edit.
- Expose explicit states such as `unavailable`, `desktop_attached` (with app instance and document identity) and `headless` (only when selected). Before every mutation, verify the selected app instance and document are still the intended targets. On disappearance, ambiguous multiple windows or changed document identity, stop and request a fresh selection. Opening the desktop must never silently leave an agent editing an unrelated blank headless document.
- Preserve legacy CLI defaults until an app migration explicitly documents a compatible new mode. In the new desktop-attach mode, failed attachment is an error, not an implicit switch to headless. A person can still deliberately request headless mode.
- Listening on loopback does not prove the caller is authorised. Do not ship auto-start with an unauthenticated editing port. Retain each app's path restrictions, destructive-operation checks and any user-owned Keep/Undo decisions.

MCP stdio clients normally own the server process and its registration. Starting the app's listener does not make an arbitrary client discover a newly installed MCP server. The agent registers the connector once; app startup and reconnect happen behind that stable registration. Clients that support another transport can be evaluated separately, without making dynamic client registration a prerequisite.

### 2. Token cost and command discovery

- Measure live `tools/list` JSON bytes and tokenizer tokens separately from command-catalogue responses and complete task transcripts. Pin app versions, mode, client, tokenizer, tasks and fixture documents. Run the same checks for all 13 apps; do not extrapolate from the historical seven-app snapshot or convert bytes to a claimed token bill.
- Default to a short, stable set of native tools per app: status/target, bounded search, describe-one-command, call-by-ID, and only those app-specific tools that merit direct exposure. Use ordered stable schemas so clients can cache them. Do not replace the app's richer operations with a generic untyped executor if that loses validation or intent.
- Search must be bounded and paginated, with a small default limit, total/more indicator, and compact entries. Reject an unbounded catalogue request or require an explicit capped limit. `describe` returns typed parameters, capability/mode and an example on demand. Derive schemas from the app's actual command metadata or make a typed registry authoritative, rather than parsing prose in a generic library. Preserve direct batch calls only where the app implements safe batch semantics.
- A curated command profile or allowlist must be **enforced by the app server** on listing, describe, call-by-ID and batch execution, and on any other command-entry surface it exposes. A compact tool list by itself does not prevent the agent from fetching or invoking the entire registry. Profile membership, unavailable commands and denied commands need distinct errors.
- Put shared limit types, pagination and schema envelopes in an optional `craft-mcp` crate only after proving two consumers can use them. Keep the per-app command tables in those apps. Prefer adding adapters to their current MCP implementations over requiring all apps to replace their transport library.

Existing evidence motivating this work is deliberately narrower than a suite benchmark: a [historical seven-app release snapshot](https://github.com/storytold/photocraft/issues/1252) found LightCraft's compact `tools/list` was 9.8 KB versus 82.5 KB full; a [PhotoCraft v0.7.0 report](https://github.com/storytold/photocraft/issues/3994) measured about 54k tokens for an unfiltered 858-command response; a [FilmCraft 0.6.0 report](https://github.com/storytold/filmcraft/issues/1545) recorded roughly 188k characters for an unfiltered catalogue. The [GridCraft startup-order report](https://github.com/storytold/gridcraft/issues/2280) reproduces the separate headless attachment failure. These are from different versions/tests, not an estimated 13-app saving. Current behaviour must be measured again.

### 3. One conversation switching between apps

- First try registering the 13 compact native servers once in the agent client, with lazy loading or deferred tool discovery where the client supports it. Switching to an already-open app then needs no MCP restart. Measure the actual tool/context cost and UX of this configuration.
- If the client cannot keep this stable or the baseline remains too expensive, build a **thin, optional suite-facing connector** outside the shared library. Its small surface selects an app/instance/document, lists only that target's permitted commands, and dispatches through the native app adapter. It never merges 13 full registries and never bypasses native auth or edit policy. Its first use must make the target explicit; an unavailable or ambiguous target is an error, not an instruction to launch an arbitrary app or create a headless document.
- A target selection is conversation/session-local, not a global mutable "active app" shared by unrelated agent clients. Read-only discovery may inspect several apps; mutations must carry or resolve an explicit target identity. Treat cross-app copying as separate reads and writes with each app's existing file and consent checks.

## Delivery sequence and acceptance gates

1. **Baseline and contract audit:** capture live `tools/list`, unfiltered/bounded command search, full representative tasks and app-switch timings for all 13 current native versions. Record client/model tokenizer and whether schemas are actually sent to model context. Record each app's auth, headless fallback, control flags, instance model, document selection and command-search path. Publish fixtures and machine-readable raw results alongside a short comparison. No percentage target until a matched baseline exists.
2. **Secure connection foundation:** finish the existing `craft-control` server and client work, including auth, descriptor ownership, bounded frames, deadlines and unknown-outcome handling. Its published contract is the prerequisite for auto-start. Pilot on two independent apps, including their desktop and MCP clients; test no-listener, bad token, stale descriptor, app restart, lost response and cross-user access. Do not weaken an already authenticated app for compatibility.
3. **Runtime attach pilot:** implement attach/status and opt-in app-launch readiness in two apps with different existing modes (proposed: LightCraft and PhotoCraft, subject to the audit). Start with `apps/lightcraft/src/control_server.rs` and `crates/mcp/src/{server,tools}.rs`, then `apps/photocraft/src/control_server.rs` and `crates/automation/src/server.rs`. Start MCP first, launch app and open a real document, then attach and prove an inspect and a deliberate edit target that document. Test multiple windows, app exit/restart, client restart, wrong document, explicit headless, denied permission and unsaved changes. Preserve old CLI behaviour during transition.
4. **Compact discovery pilot:** compare the two apps' MCP sources; extract only demonstrated common contracts to `crates/mcp/` as optional `craft-mcp`, register it in root `Cargo.toml`, document it in `crates/mcp/README.md` and link it from root `README.md`. Add crate-level tests for stable ordering, bounded output, invalid schema/params, disabled profile calls and pagination. Test both apps against real client `tools/list` and representative tasks. No dependency on an app crate.
5. **Suite rollout:** migrate the other 11 apps incrementally with an app-specific compatibility checklist. Keep native command semantics and review each opt-in UI, security and mode fallback. Use one app PR at a time; if a second consumer cannot use an abstraction without invasive app changes, keep it app-local rather than forcing uniformity.
6. **Switching gate:** benchmark all-registered native servers against an optional single suite connector. Build the connector only if client limitations or measured context/UX justify it. Run a same-conversation workflow across at least three different open apps, with an app closed and reopened mid-session, and check that edits never reach a stale or headless target.

For code changes, follow `craft-libs` gates (`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`) in CI; run targeted crate and app tests during development. Performance acceptance compares matched tasks: `tools/list` and search tokens, calls, task-success rate, wrong-target/permission failures, and startup/switch latency. Report absolute figures and matched deltas; a smaller tool list is not success if discovery costs or task failures increase.

## Decisions requested in review

1. Does `craft-control` own all endpoint/auth/reconnect primitives, leaving the MCP crate to discovery and result shaping?
2. Should desktop attach be the new opt-in profile while existing no-flag mode remains compatible until each app migrates?
3. Which two native apps should prove the shared boundary first, and which app command-profile policy is acceptable?
4. Do supported agent clients make 13 compact native registrations workable, or does a measured need justify a separate suite connector?

**Out of scope for this docs PR:** code, a production-facing listener, a claim of achieved token savings, a mandatory universal command registry, and changes to the existing Ask Agent or AI model connection contracts.
