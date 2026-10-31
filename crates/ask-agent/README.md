# craft-ask-agent

Ask Agent for the Craft apps: the person selects something, types an instruction, and **the agent they
already run** (Claude Code, Codex, any MCP client) does it in the document. The app calls no model and
holds no key.

## What is shared

- `hub` (always): the request queue (`ask`, `listen`, `reply`, `close`), the agents listening (`agents`,
  `colour`), the one edit rule (`may_edit`: while a request is open only the connection working on it
  edits, and only for it), what is held (`held`), and the control methods `agent.listen` /
  `agent.reply` (`AgentHub::handle`, called from the control channel's connection threads). Refusals
  carry stable codes: `HELD`, `NOT_OWNER`, `NO_REQUEST`, `INVALID_PARAMS`, `READ_ONLY_REQUEST`.
- `ui` (feature `egui`): `chip` beside the selection and `bubble` (instruction, agent picker, progress
  notes, answer, Keep / Undo / follow-up, and a compact Actions menu). egui's visuals only; the app passes its translation function.

## What stays in each app

The app adapter supplies the selection and document as JSON, grouping the agent's edits into one undo step,
authoring them "Agent (name)", refusing the person's edits that touch a held target, and a closing
comment with the agent's answer.

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

- `agent.listen {timeoutMs?, agent?}` → `{id, instruction, history, selection, document, agent}` or `{none: true}`
- `agent.reply {id, text, done?, error?}` → `{id, done}`
- MCP tools in each app: `wait_for_request`, `reply_to_request`.

Used by WordCraft, GridCraft, DeckCraft and PdfCraft.
