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

A task keeps one assignee from start to finish: review fixes and later follow-ups go back to the same person or agent.

### 6.1 Brief

The brief is complete before work starts: the map sections, the app files, the framework components with their files, the test verdicts, the states to compare, the done-when, and any decisions already made. If the brief leaves a design question open, the assignee stops and asks rather than guessing.

### 6.2 Plan checkpoint

Before editing, the assignee writes a short plan: the mechanism, the files, and how the result will be checked. A reviewer (the lead, or a second agent) checks it against the map and the rules below. A plan that adds a framework mechanism, or departs from the map, needs the lead's answer before code is written.

### 6.3 Write and prune

- Write the new host code on the framework components and move the callers.
- Delete the app code the map replaces, and the tests marked `delete`, with a one-line reason each. No old code stays beside the new.
- A fix that adds a mechanism removes the one it replaces in the same change.

### 6.4 Compare old and new

The old app is the visual reference, but not automatically right: judge each difference on its own (old better, new better, or both need work).

- Script each state so it can be reproduced: a state is a reset, then steps sent through the app's control channel ([craft-automation](../crates/automation/README.md)), such as opening a panel, setting a value, opening a dialog. Keep the states in a file per panel or dialog.
- Capture each state in the old app and in the port, side by side, at the same size and theme.
- Shapes, colours, borders, glyphs and parts match; exact pixel values may differ. Nothing overlaps or cuts into a neighbour.
- A difference from the old needs a reason: a standardized framework mechanism, or one way replacing an old inconsistency.
- Repeat until the done-when states match.

#### What the library gives and what the app adds

`craft-automation` provides the transport, the authentication, the request queue, the screenshot queue and a few methods. The app provides the method table.

| Part | Library (`craft_automation`) | App |
|---|---|---|
| Command-line values | `args::ControlOptions`, `EnvNames` | Parses its own flags and calls `ControlOptions::resolve` |
| Transport | `server::start`, `ControlClient` | Calls `start`, owns the `ControlChannel` |
| Request queue, replies | `ControlChannel`, `Reply`, `result_reply` | Runs each request and returns a `Reply` |
| Input methods | `synthetic::Click`, `KeyPress`, `move_from_params`: the checked parameters of `ui.click`, `ui.key`, `ui.move` | Dispatches the method names to them |
| Dialog methods | `dialogs`: `ui.dialog.set`, `ui.dialog.confirm`, `ui.dialog.cancel` and the dialog list | Dispatches them; may add `ui.dialog.open` for its own dialog kinds |
| Screenshots | `Captures` (the bounded queue), `screenshot_reply`, PNG encoding | Allocates a token for `ui.screenshot`, drives the queue each frame, supplies the file writer |
| View and window state | nothing | Methods such as `ui.set` (tool, panels, theme), `ui.resize`, `engine.execute` |

Not provided yet, so the porter supplies each today:

- a capture tool: a small client program (the sample below) that reads a states file, sends the steps and saves the PNGs;
- a states-file format and its runner (the sample below defines one);
- window-size, theme and reset methods: each app defines its own (PhotoCraft: `ui.resize`, `ui.set {theme}`);
- a picture comparison: place the two PNGs side by side with any image tool.

#### Expose the control channel in the app

The app starts the server once and keeps the channel on its UI thread.

```rust
use craft_automation::args::{ControlOptions, EnvNames};
use craft_automation::control::ControlChannel;
use craft_automation::server;
use craft_ui::runtime::windows::WindowHost;

// At start: `flags` holds what the app's own parser read for --control, --control-token
// and --control-token-file (a value of the right type, or None).
let names = EnvNames::for_app("MYAPP"); // MYAPP_CONTROL_PORT, MYAPP_CONTROL_TOKEN, ...
let options = ControlOptions::resolve(flags, &names)?;
if let Some(port) = options.port {
    let token = options.server_token()?;
    eprintln!("myapp: {}", options.token_notice(&token));
    // Port 0 lets the OS pick; the address is the one a client needs.
    let (requests, addr) = server::start(port, token, WindowHost::of(&egui_ctx))?;
    eprintln!("myapp: control server listening on {addr}");
    app.control = Some(ControlChannel::new(requests));
}
```

Each frame, in the logic phase, the app runs the queued requests. In the UI phase of a real pass it issues due screenshots; in the logic phase it collects finished ones (PhotoCraft's `crates/ui-egui/src/app/control.rs` on the `spike/framework-sample` branch is a complete example).

```rust
use craft_automation::control::{Reply, result_reply};
use craft_automation::screenshot::{CaptureGeometry, settle_ms};
use craft_ui::runtime::motion;

// Logic phase.
let Some(mut channel) = self.control.take() else { return };
let document = self.document_identity(); // None when the app has no document identity
channel.captures.note_geometry(CaptureGeometry::from_facts(WindowHost::of(ctx).screenshot_facts(), document));
let observed = channel.captures.last_geometry();
let finished = channel.captures.drain(motion::now(), observed, document);
channel.finish_captures(finished, &mut |path, bytes| self.write_automation_file(path, bytes));
let settle = settle_ms(ctx.global_style().animation_time);
while let Some(req) = channel.next_request() {
    let reply = match req.method.as_str() {
        "ui.screenshot" => Reply::screenshot(self.next_capture_token(), path_param(&req.params)),
        "ui.resize" => { /* send ViewportCommand::InnerSize */ Reply::from_result(Ok(json!(null))) }
        "ui.set" => Reply::from_result(self.ui_set(&req.params)),
        other => Reply::from_result(Err(format!("unknown method {other}"))),
    };
    channel.settle(req, reply, document, settle);
}
self.control = Some(channel);

// UI phase of a real pass: count it for settling, record the geometry, then issue what is due.
channel.captures.note_pass();
let finished = channel.captures.issue(&WindowHost::of(ctx), motion::now(), document);
```

`write_automation_file` is the app's file authority: it must refuse paths outside the folder the app lets automation write to. A `ui.screenshot` without a `path` returns the PNG as base64 in the reply.

#### States file

A state is a title and steps. Each step is one request: a method and its parameters. The methods below are PhotoCraft's (`ui.set`, `ui.resize`, `ui.dialog.open`, `engine.execute`) and the library's (`ui.click`, `ui.key`, `ui.dialog.*`); use the ones the app registers.

```json
{
  "size": { "width": 1280, "height": 800 },
  "theme": "pro",
  "states": [
    { "id": "default", "title": "Default", "steps": [] },
    { "id": "pencil", "title": "Pencil tool", "steps": [
        { "method": "ui.set", "params": { "tool": "Pencil" } } ] },
    { "id": "about", "title": "About dialog", "steps": [
        { "method": "ui.dialog.open", "params": { "kind": "about" } } ] },
    { "id": "click", "title": "Pointer at a canvas position", "steps": [
        { "method": "ui.click", "params": { "x": 640, "y": 400 } } ] }
  ]
}
```

Before each state the client sends the same two set-up requests (`ui.resize`, then `ui.set {theme}`), so both apps render at one size and theme. A state that leaves a dialog open ends with its `ui.dialog.cancel` before the next one.

#### Take the captures

The client connects with `ControlClient`, sends each state's steps and asks for a capture with `ui.screenshot {path}`. The `path` is relative to the folder the app lets automation write to.

```rust
use craft_automation::client::ControlClient;
use serde_json::{Value, json};

/// Sends one state of the states file and saves its capture as `<label>-<id>.png`.
fn capture_state(client: &mut ControlClient, doc: &Value, state: &Value, label: &str) -> Result<Value, String> {
    let set = |client: &mut ControlClient, method: &str, params: Value| client.call(method, params).map(|_| ());
    set(client, "ui.resize", doc["size"].clone())?;
    set(client, "ui.set", json!({"theme": doc["theme"]}))?;
    for step in state["steps"].as_array().into_iter().flatten() {
        client.call(step["method"].as_str().ok_or("a step needs a method")?, step["params"].clone())?;
    }
    let id = state["id"].as_str().ok_or("a state needs an id")?;
    client.call("ui.screenshot", json!({"path": format!("{label}-{id}.png")}))
}
```

The reply is `{"path", "width", "height"}`. Run it once per app: start the old app and the port each with `--control 0` and a token file, read the address each prints (`control server listening on 127.0.0.1:<port>`), connect, and call `capture_state` for every state with the labels `old` and `new`. Check that both replies give the same `width` and `height`; a different size means the window did not reach the requested size, and the pair is not comparable.

Send the steps with `ControlClient::call`: a call returns after the input it queued was processed, so the next step sees its result. After a state's steps, `ui.screenshot` waits until open and close animations settle before it captures.

### 6.5 Tests

- Rewrite the `keep` and `table` tests on the framework. Event mappings become a few table-driven tests per event family.
- Test behaviour that could regress, not exact sizes.
- Framework changes get trials in their own crate (`cl test <crate> -- <filter>`).

### 6.6 Review and merge

- A reviewer reads the diff and the last old/new comparison, then passes the task or lists fixes. Fixes go back to the same assignee; a fix that restructures gets reviewed again.
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
2. Run the app's whole test suite once, and sort each failure to the task that caused it; it goes back to that task's assignee.
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

The file format, the methods and the runner are in [6.4](#64-compare-old-and-new). One state:

```json
{ "id": "tip-open", "title": "Tip shape open", "steps": [
    { "method": "ui.set", "params": { "tool": "Brush" } } ] }
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
