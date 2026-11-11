# craft-automation

Drive a running Craft app from tests, agents and the CLI, and record and play actions. The control channel, its transport, screenshots, the profiler and the trace are desktop only; `actions` and `ui::actions` build on every target, the browser included.

| Module | What it gives the app |
| --- | --- |
| `actions` | Recorded actions (every target): `ActionList` of `Action`s and `ActionStep`s, `ActionsFile`, playback with `ActionPlayer` and `ActionRunner` under a `PlaybackBudget`, and `Recording`. The app keeps the command table. |
| `ui::actions` | `ActionsPanel` (every target): sets, actions and steps as a tree, include and dialog cells, rename (`start_rename`), drag to move, Button Mode, the footer and the menus (`ActionsPanelCommands` and `panel_rows`). It runs nothing; it reports `ActionsEvent`s. `ActionsPanelState` holds its view state. `ActionsWords` holds its words. |
| `args` | `ControlOptions`: the `--control <PORT>`, `--control-token`, `--control-token-file` values and their `<APP>_CONTROL_*` environment fallbacks, checked the way the apps' command lines report them (usage errors exit 2). |
| `security` | Tokens (`generate_token`, `validate_token`, `server_token`, `client_token`), the `auth` handshake, bounded line reads, connection and request limits. |
| `protocol` | The JSON-lines wire format: one `{"id", "method", "params"}` request per line, one `{"id", "ok", "result" | "error"}` reply per line; `Params` validators that name the bad parameter. |
| `control` | `ControlRequest`, `Reply`, `Dispatch` and `ControlChannel`: the UI-thread side that drains requests between frames, replies after injected input or a background job settles, and admits screenshots. The app supplies the method table. |
| `synthetic` | `Click`, `KeyPress` and `move_from_params`: the validated parameters of `ui.click`, `ui.move` and `ui.key`, queued through `WindowHost::inject_*` one step per pass. |
| `server` | `start`: the loopback TCP listener that authenticates each connection and forwards requests to the channel. |
| `client` | `ControlClient`: connect, authenticate, call, screenshot; for integration tests and CLIs. |
| `screenshot` | `Captures`: the bounded, cancellable screenshot queue over `WindowHost::screenshot`, with settling, geometry epochs, retries and the PNG reply. |
| `profiler` | `FrameStages` and `SlowFrames`: where a frame's time went, the slow ones kept for the control channel's `perf` output and one log line a second. |
| `trace` | `FrameTrace`: every frame of a gesture (a brush stroke, a drag) as a row, a percentile summary per gesture, optional CSV. |

The app owns its methods (`engine.execute`, `ui.inspect`, `ui.set`, …): `ControlChannel::next_request` hands it each request and `ControlChannel::settle` takes the `Reply`. The library owns everything a method table does not: deadlines, dispatch accounting, waiters, the screenshot budget, injected input and the transport.
