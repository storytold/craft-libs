# Porting an app to the Craft framework

This guide is for developers, and the agents they brief, who move a craft app's UI onto the Craft framework on craft-libs' `next` branch. It describes the whole workflow: from finding what the app has, through mapping it to framework components, to merging the port. It assumes no particular machine or tooling; where a step needs a tool, it says what the tool must do.

Read first:

| Doc | For |
|---|---|
| [craft-framework.md](craft-framework.md) sections 1–4 | What the framework is, its crates, layers, and how an app hosts it |
| [authoring.md](authoring.md) | Writing a component, its stories, and checking it |
| [components-index.md](components-index.md) | Every component that exists, with its shape and purpose |
| [ui-dialogs.md](ui-dialogs.md), [ui-docking.md](ui-docking.md) | Dialogs and the dock, which most ports touch early |
| [contract-api.md](contract-api.md) | The public API surface |

## 1. The workflow at a glance

| Phase | Output | Section |
|---|---|---|
| Pin | The app builds against a `next` revision | [2](#2-branches-and-pins) |
| Inventory | Every user-visible part of the app's UI, with its code and tests | [3](#3-inventory) |
| Map | For each part: the framework component, the gaps, the test verdicts | [4](#4-map) |
| Plan | Tasks with dependencies, sizes and a done-when each | [5](#5-tasks) |
| Port | One task at a time: plan, write, prune, compare, tests, review, merge | [6](#6-porting-one-task) |
| Integrate | Repin, the app's full test run, a check on each platform | [7](#7-integration) |

Framework work found on the way (a missing component, a missing option) is its own task on `next`, merged before the app task that needs it.

## 2. Branches and pins

- The framework lives on `next` in storytold/craft-libs. Apps keep pinning `main` until they port.
- A port branch in the app pins one `next` revision by git rev. Move the pin forward deliberately (a "repin"), never to a moving branch name.
- Framework changes go to `next` as normal PRs. Do not patch the framework inside the app.
- `next` is synced with `main` regularly. When `main` changes code that `next` replaces, the sync ports the change across.

## 3. Inventory

List every user-visible part of the app's UI: panels, dialogs, bars, menus, canvas overlays, popups, and shared widgets. For each part record:

- the app files and line ranges that build it
- the test files that cover it, with their test counts
- what the user does with it (the states worth seeing: empty, filled, open, disabled, error)

Split a large app into areas (for example: panels and chrome, dialogs, tools, canvas, menus and settings) so each area's inventory can be done separately. An agent can draft the inventory from the source; a person checks that nothing user-visible is missing.

## 4. Map

For each inventoried part, decide what it becomes. Record:

| Field | Meaning |
|---|---|
| Framework replacement | The component(s) that replace it, from components-index.md |
| Fit | `direct` (use as is), `needs props` (the component lacks options; list them), `missing component` (describe it), `app-only` (stays app code, built from framework components) |
| Gaps | Framework changes the part needs, each a `next` task |
| App glue | What the app still does: commands it sends, state it owns, settings it saves |
| Test verdicts | Per test file: `delete` (the framework now owns the behaviour, or the code goes), `table` (event-to-command mapping, folded into a few table-driven tests per event family), `keep` (the app's own decisions: command ids, engine effects, saved settings), `new` |
| States | The states to compare old against new (section [6.4](#64-compare-old-and-new)) |
| Depends on | Other parts and gaps that must land first |
| Size | S, M or L in agent tool calls (S under about 40, M up to about 100, L up to about 150; split anything larger) |

Gaps that several parts share are listed once, as shared framework gaps, so they are built once.

### Reuse before adding

Before marking a part `missing component`, walk this ladder and stop at the first step that works:

1. **An existing component as is.** Search components-index.md by purpose and by shape, and look at the component's stories in the gallery.
2. **An existing component with a new story or variant.** If the component already does the job and only lacks a shown state, add the story (authoring.md, "Stories"). A story is a state of an existing type, not a new type.
3. **A builder option on an existing component.** A structural difference gets a new option on the component, not a new component (craft-framework.md 4.6, step 4).
4. **Density, size overrides or component tokens.** Visual differences an app needs are tokens and size overrides, never component-specific theme values (craft-framework.md 4.6, steps 1–3).
5. **Per-app colour overrides** in the theme file (craft-framework.md 4.6, step 5).
6. **A family component** in the crate that owns the document concept, when the part names one (layer, colour model, path): craft-framework.md 4.3.
7. **A new library component**, when its inputs are general data and no existing component fits. Place it in the lowest crate of its layer (craft-framework.md 4.3).
8. **A one-off app component**, built from shared pieces, as a last resort.

If two apps need the same family component, it moves down to the library once its inputs are general.

### Adding a component or story to the gallery

The gallery shows every registered story; there is no separate gallery list to edit.

- A new component is registered by its type (`#[derive(Component)]` or a pattern) and gets its required stories (`Default`, plus `Mixed`, `Invalid`, `LongText`, `Open`, `Empty`/`One`/`Many` as its probe requires): authoring.md, "Stories". The shared suite fails while a required story is missing.
- A variant of an existing component is a story with a `title`, grouped with the type's other stories by `group`.
- The component's gallery level follows its architectural kind (primitive, control, composite, pattern, shell), not the app it came from.
- After adding or renaming a type, regenerate components-index.md (`cargo run -p craft-ui-gallery --bin components-index`) and commit it.
- Check the result with `render` (the stories to PNG files, without the gallery) or the gallery itself: authoring.md, "Seeing and checking the result".

## 5. Tasks

Turn the map into tasks:

- One task per map section (one part, or a few small parts that share a host). Each task fits one agent run of about 150 tool calls; split larger ones.
- Each task names its sections of the map, its dependencies, and a **done-when**: the states that must match, the tests rewritten, the old code removed.
- Framework gaps are their own tasks, scheduled before the app tasks that need them.
- Order by the dependency graph; the longest chain (often the canvas or the dock) is the critical path and starts first.

## 6. Porting one task

A task keeps one owner from start to finish: review fixes and later follow-ups go back to the same person or agent.

### 6.1 Brief

The brief is complete before work starts: the map sections, the app files, the framework components with their files, the test verdicts, the states to compare, the done-when, and any decisions already made. If the brief leaves a design question open, the owner stops and asks rather than guessing.

### 6.2 Plan checkpoint

Before editing, the owner writes a short plan: the mechanism, the files, and how the result will be checked. A reviewer (the lead, or a second agent) checks it against the map and the rules below. A plan that adds a framework mechanism, or departs from the map, needs the lead's answer before code is written.

### 6.3 Write and prune

- Write the new host code on the framework components and move the callers.
- Delete the app code the map replaces, and the tests marked `delete`, with a one-line reason each. No old code stays beside the new.
- A fix that adds a mechanism removes the one it replaces in the same change.

### 6.4 Compare old and new

The old app is the visual reference, but not automatically right: judge each difference on its own (old better, new better, or both need work).

- Script each state so it can be reproduced: a state is a reset, then steps sent through the app's control channel (`craft-automation`), such as opening a panel, setting a value, opening a dialog. Keep the states in a file per panel or dialog.
- Capture each state in the old app and in the port, side by side, at the same size and theme.
- Shapes, colours, borders, glyphs and parts match; exact pixel values may differ. Nothing overlaps or cuts into a neighbour.
- A difference from the old needs a reason: a standardized framework mechanism, or one way replacing an old inconsistency.
- Repeat until the done-when states match.

### 6.5 Tests

- Rewrite the `keep` and `table` tests on the framework. Event mappings become a few table-driven tests per event family.
- Test behaviour that could regress, not exact sizes.
- Framework changes get trials in their own crate (`cl test <crate> -- <filter>`).

### 6.6 Review and merge

- A reviewer reads the diff and the last old/new comparison, then passes the task or lists fixes. Fixes go back to the same owner; a fix that restructures gets reviewed again.
- Merge the framework branch to `next` first, then the app branch onto the port branch.

### 6.7 Rules the review checks

- One mechanism per concept; one set of components.
- No workarounds or quick fixes; make the fix where it belongs, in any crate.
- Use the framework's own signals; never infer a fact from side effects. A missing signal is added where it belongs.
- Behaviour follows the app's conventions; the framework shares focus, Tab order, Escape, undo steps, themes and translation.
- Dialog footer buttons follow the platform order (OK last on macOS and Linux, first on Windows).

## 7. Integration

At the end of a batch of tasks (a "wave"):

1. Repin the port branch to the `next` head that holds the batch's framework merges, and run a compile check.
2. Run the app's whole test suite once, and sort each failure to the task that caused it; it goes back to that task's owner.
3. Build and run the app on each platform it ships on, and check what the batch changed by eye. Platform-only behaviour (GPU drawing, cursors, window chrome) is checked here.
4. Record what the batch cost (time, tool calls, review rounds) and change the process where it went wrong.

## 8. Working with agents

- Brief each agent with one task and its map sections, not the whole map.
- Tell the agent how to test: the narrowest check while writing, trials of the crates it changes, and no workspace-wide builds. The full test run belongs to integration.
- Ask for a plan before edits and answer it before the agent writes.
- Have the agent keep a report with a **Resume** section (the step, what is done, the next command) and push its branches at every step end, so a fresh agent can continue after an interruption.
- One reviewer per task, at the end, not per round.

## 9. Templates

### Map section

```markdown
### A4 Brush Settings panel

- **App code**: panels/brush_panel.rs (all), panels/brush_preview.rs:1-240
- **Framework**: `Section`, `ListRow`, `Slider`, `CheckRow`; preview through `RasterDisplay`
- **Fit**: needs props: `ListRow` label column (gap G-12)
- **Gaps**: G-12 ListRow label column
- **App glue**: sends `brush.set` commands; owns the brush settings model and its saved presets
- **Tests**: brush_panel/tests.rs (42): 30 table (event family "slider edits"), 8 keep (preset save/load), 4 delete (preview cache: the framework owns previews)
- **States**: default; tip-shape section open; dual-brush on; long preset name
- **Depends on**: G-12
- **Size**: M
- **Done when**: the four states match the old panel; preview cache code and its tests are gone; kept tests rewritten
```

### State file

```json
[
  { "id": "default", "title": "Default", "steps": [ { "method": "ui.panel.open", "params": { "panel": "brush-settings" } } ] },
  { "id": "tip-open", "title": "Tip shape open", "steps": [
      { "method": "ui.panel.open", "params": { "panel": "brush-settings" } },
      { "method": "ui.click", "params": { "label": "Brush Tip Shape" } } ] }
]
```

The method names come from the app's control channel; use the ones it registers.

### Task report

```markdown
# <task id>: <title>

## Resume
- Step: <brief | plan | write | prune | compare | tests | review>
- Done: <what is finished, with commits>
- Next: <exact next command or action>

## Plan
<mechanism, files, checks>

## Progress
- <time> <one line per step>

## Differences from the old app
- <state>: <difference> - <reason: old better / new better / both need work, and the target>

## Open
- <questions for the lead, follow-ups>
```
