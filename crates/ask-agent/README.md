# craft-ask-agent

Ask Agent for the Craft apps: the person selects something, types an instruction, and **the agent they
already run** (Claude Code, Codex, any MCP client) does it in the document. The app calls no model and
holds no key.

## What is shared

- `hub` (always): the bounded request queue (`ask`, `listen`, `reply`, `close`), the agents listening (`agents`,
  `colour`), the edit guard (`may_edit`: owning connection, current document/revision, and an app-owned
  selection predicate), what is held (`held`), and the control methods `agent.listen` /
  `agent.reply` (`AgentHub::handle`, called from the control channel's connection threads). Refusals
  carry stable codes: `HELD`, `NOT_OWNER`, `NO_REQUEST`, `INVALID_PARAMS`, `READ_ONLY_REQUEST`,
  `STALE_REQUEST`, `OUTSIDE_SELECTION`, `LIMIT_EXCEEDED`.
- `ui` (feature `egui`): `chip` beside the selection and `bubble` (instruction, agent picker, progress
  notes, answer, Keep / Undo / follow-up, and a compact Actions menu). egui's visuals only; the app passes its translation function.

## What stays in each app

The app adapter supplies the selection and document as JSON, grouping the agent's edits into one undo step,
authoring them "Agent (name)", refusing the person's edits that touch a held target, and a closing
comment with the agent's answer.

## Document and selection contract

The queue owns request assignment; it cannot infer a spreadsheet range, text interval, slide object
or PDF rectangle from arbitrary JSON. Each app must wire the following checks into **every** mutation
path, including commands addressed to a non-active document:

1. Capture `Request::target` from the document owner, with a stable open-document identity and a
   monotonically increasing content revision. A filename or active-tab index is not an identity.
   Reopening or replacing a document allocates a new identity; undo and redo advance the revision.
2. Handle the fallible `ask` and `listen` results. A full queue or oversized payload is refused;
   nothing is silently dropped, truncated or evicted. Connection ids are unique for their lifetime.
3. On the app's document thread, call `may_edit(conn, request, current, contains)` with the actual
   current target and an app-owned predicate that checks the **proposed command** against the
   captured selection. Do not accept the agent's claimed selection or revision as proof. A prompt
   saying "only edit these cells" is not an access check.
4. Serialize authorization and execution on that document thread. After a successful agent edit,
   call `advance` with the previous target, new revision and rebased held selection. This allows
   multiple owner edits while invalidating other requests for the old revision. If advancement
   fails, invalidate the request against the actual new revision; do not continue with a stale guard.
5. Notify `invalidate` after any external edit or replacement. For progress and final replies,
   the `current` resolver passed to `handle` must look up the **captured document**, not the active
   tab, and return its current target or `None` if it closed. Direct `reply` calls supply that same
   trusted current target. Resolve and accept replies in the same serialized document operation;
   a socket thread must dispatch this work to the document owner rather than reading a token and
   accepting the response later. Stale requests fail and their late response text is not retained.
6. Keep/Undo, command-specific access checks, selection rebasing and atomic undo ownership remain
   in the app. This crate does not make an arbitrary command safe merely because its owner matches.

The tests use a synthetic cell interval to exercise the shared predicate contract. Each application
must additionally test its real command routing, geometry and document mutation handling.

## Resource limits

`AgentHub::default()` retains at most 128 requests (including completed requests until closed),
1 MiB of serialized input per request, 256 KiB per response envelope, 128 progress notes and 64 KiB of
progress text per request, and 64 registered connections. Agent names/document identities are
limited to 256 bytes. `with_limits` accepts smaller or larger app-owned bounds.

Response text and errors are limited to 4,000 Unicode characters each, a proposal to 200,000 bytes,
a comment to 2,000 Unicode characters and format metadata to 64 bytes. These per-field limits also
apply when a larger envelope limit is configured. A proposal-only final response is accepted;
`proposal`, `format` and `comment` are retained in `Snapshot`, including alongside ordinary text.

Size checks count serialized bytes without allocating another document copy. Over-limit messages
return `LIMIT_EXCEEDED` without partially storing them. Closing a request releases its capacity;
disconnecting a connection releases its listener slot and requeues only its still-active work.

## Untrusted text

Instructions, prior exchanges, progress, errors, answers and agent names are displayed with egui
labels/`RichText`, which render strings as literal text. They are never fed to a Markdown/HTML
renderer, URL launcher or command executor. `tests/ui.rs` checks literal markup-shaped strings and
that rendering them issues no platform command. Embedding apps must preserve that rule in native
comments, logs and alternative renderers too.

A proposal is review text, even when its `format` says JSON or Markdown. The shared bubble labels
it **Proposal (not applied)** and never executes or parses it. Keep preserves only edits already
performed through authorized native tools and the app's saved response note. Apps can use
`Snapshot::review_text()` to retain the answer, comment, format and unapplied proposal together;
accepting the note must never turn the proposal into a command.

## App-specific action shortcuts

`ui::View::actions` accepts a slice of `ui::QuickAction { label, group, instruction }`.
An empty `group` places an item at the menu root; other values form native submenus.
Choosing an action only fills the editable instruction. The person still presses Send.
The app owns its presets and their wording; the shared UI adds no document operation or
permission. Prompts are suggestions, not enforcement of read-only or signing policy.

## Visible conversation

`ui::View::history` contains the prior instruction/answer pairs and `instruction` is the current request.
The bounded, collapsible Conversation panel exposes these exchanges in the bubble without changing
the wire protocol. Each app records the instruction with the agent's answer in its native comments
or notes so that the context remains after Keep. This is conversation context, not document version history.

## Wire protocol

- `agent.listen {timeoutMs?, agent?}` → `{id, instruction, history, selection, document, target, agent}` or `{none: true}`
- `agent.reply {id, text?, done?, error?, proposal?, format?, comment?}` → `{id, done}`
- MCP tools in each app: `wait_for_request`, `reply_to_request`.

Used by WordCraft, GridCraft, DeckCraft and PdfCraft.

## Checks

`cargo fmt -p craft-ask-agent --check`,
`cargo clippy -p craft-ask-agent --all-targets --all-features --no-deps --locked -- -D warnings`,
`cargo test -p craft-ask-agent --no-default-features --locked` and
`cargo test -p craft-ask-agent --all-features --locked` cover the crate. The path-filtered
`Ask Agent` workflow runs these on Linux/Windows and checks the all-feature WASM build. It runs on
matching pushes and pull requests; maintainers can also start it with `workflow_dispatch`.

The four app PRs still carry copies of the previous implementation until migrated. To adopt this
crate they must supply `Target`, handle the fallible queue API, implement the selection predicate,
advance/invalidate revisions and resolve the captured document for replies. Merely replacing a
dependency without these changes does not deliver the guarded behavior described above.
