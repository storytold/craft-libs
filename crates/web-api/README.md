# craft-web-api

The page command API for every Craft web build: one `window.<app>` shape over the app's JSON
control channel.

## The JS API

`PageApi::install` puts this on `window.<app>` (`"vectorcraft"` → `window.vectorcraft`), without
replacing an existing `window.<app>`: missing functions are added, an app that already has its own
keeps them. Each function returns a Promise that resolves with the method's `result` or rejects with
an `Error` carrying `error`. Every call has a timeout (30 s by default, `PageApi::with_timeout`);
when the app does not answer in time, the error says so and the app's request future is dropped, so an app that releases a request when its future goes away (the desktop `ControlRequest` has `abandon`) never runs a call the page was told had failed.

`screenshot()` resolves with the reply the app's control method sends, plus `data` when the reply carries the PNG inline: the base64 PNG, whatever field the app names it (`pngBase64`, `png_base64`, `png` or `base64`). A reply with no inline PNG, such as one the app wrote to a `path`, has no `data` and is unchanged.

Which functions are installed follows the app's `Methods` table: only the functions whose control
method the table provides are installed; `request` is always there.

| Function | Call | Table entry | Default method |
|---|---|---|---|
| `request` | `request(method, params)` | — | — (as given) |
| `execute` | `execute(command, params)` | `execute` | `engine.execute {command, params}` |
| `commands` | `commands()` | `commands` | `engine.commands {}` |
| `docInspect` | `docInspect(params)` | `doc_inspect` | `document.inspect` |
| `uiInspect` | `uiInspect(params)` | `ui_inspect` | `ui.inspect` |
| `screenshot` | `screenshot(params)` | `screenshot` | `ui.screenshot` (with `data: true`); an inline PNG is always also in `data` |

`Methods::default()` is the desktop control-server method names; an app whose control channel
names a method differently, or lacks one, says so in its table (`Methods::NONE` is the empty table).

## How an app wires it

The app hands the crate its control channel as a `Request`; the crate never knows the app's
internals. Using VectorCraft's channel (the same one the desktop control server uses) as the example:

```rust
use std::rc::Rc;

use craft_web_api::methods::Methods;
use craft_web_api::page::{PageApi, Request};

// Send a ControlRequest to the app and await its reply object ({ok, result} / {ok: false, error}).
let request: Request = Rc::new(move |method: String, params: serde_json::Value| {
    let (req, rx) = ControlRequest::new(method, params);
    control_tx.send(req);
    Box::pin(poll_reply(rx)) // resolves when the app answers the request
});
let page = Rc::new(PageApi::new("vectorcraft", Methods::default(), request));
page.install()?; // window.vectorcraft
```

## Beside the host messages

Opening a document, handing a saved or exported file to the host, the document name and the dirty
flag stay with the host messages in [`craft-web-host`](../web-host/README.md) (`<app>:open`,
`<app>:save`, `<app>:dirty`, …) and the app's own control methods. A page reaches any control
method through `request(method, params)`.

## Tests

`src/methods.rs` carries the native unit tests (the default and empty tables). `tests/page.rs` runs
under `wasm-bindgen-test` in Node with a canned control channel: install and keeping an app's own
functions, the table deciding which functions exist, the method and params each function sends,
error replies as rejections, and the timeout.
