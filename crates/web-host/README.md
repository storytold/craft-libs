# craft-web-host

The host messages between a Craft app's web build and the page that hosts it, as one contract and one bridge for every app (storytold/craft-libs#358). A page frames the app with `?host=parent`, or runs it directly with `data-host="page"` on its canvas; either way the same messages pass files in, saves out, and unsaved-changes state, so a page can host any of the apps the same way.

The crate has two parts:

- **The contract** (all targets): the message kinds and their `<app>:<kind>` types, the contract version, how the hosting mode is read from the URL and the canvas, and the clean-up of file names a host passes in. Pure, tested natively.
- **The bridge** (`web`, wasm32 only): `Host::connect(app, canvas)` finds the host, `listen` hears its messages, and `post_ready`, `post_save`, `post_dirty`, `post_failed` and `post_handoff` answer it. Only the parent window is heard and answered, and only on its own origin: the app's origin, or one the app's page lists in `data-host-origins` (below). With a parent on any other origin the app runs as if unframed.

## Messages

| Kind | Direction | Fields | Meaning |
|---|---|---|---|
| `ready` | app → host | `version`, `appVersion?`, `file?` | the app can take a document; `file: true` means it reads a `File`/`Blob` in slices |
| `host` | host → app | `version`, `handoff?` | what the host supports; `handoff: true` means it routes hand-offs |
| `open` | host → app | `name`, and `bytes` or `file` + `size` | open a document as File › Open would |
| `save` | app → host | `name`, `bytes`, `id?`, `saveAs?` | Save, Save As or Export: the host stores the file |
| `saved` | host → app | `id` | the save with that `id` is stored; the app clears dirty |
| `dirty` | app → host | `dirty` | unsaved changes appeared or went away |
| `failed` | app → host | `error`, `id?` | a document or a save could not be taken |
| `handoff` | app → host | `name`, and `bytes` or `file` + `size` | this file belongs to another app |
| `handoff-done` | host → app | `name`, `app` | the host gave it to the app that edits it |

With `?host=parent` a message is an object with `type` (`pdfcraft:open`) and the fields, sent with `postMessage` to the parent's origin only; `bytes` go as a transferred `ArrayBuffer`. With `data-host="page"` it is a `CustomEvent` of that type on the canvas, with the fields in `detail`.

## Framing from another origin

By default only a page on the app's own origin can host it with `?host=parent`. A deployment can let other origins frame the app, such as a code editor that shows the app in a panel (VS Code webviews are `vscode-webview://…`, and code-server or a cloud workspace serves them on its own host). The deployment lists those origins on the app's canvas, in the HTML page its server sends:

```html
<canvas id="the_canvas_id" data-host-origins="https://*.code.example.com vscode-webview://*"></canvas>
```

- Origins are separated by spaces or commas. Write each one as the browser reports it: the scheme and host, a port only when it is not the default, no path and no trailing `/`, and international names in punycode (`xn--…`).
- A `*` may only start the host, and it stands for one or more whole labels. `https://*.code.example.com` allows `https://ws-1.code.example.com` but not `https://code.example.com`, another scheme or another port. `https://*example.com`, `https://editor.*` and `https://*` allow nothing.
- VS Code in a browser (vscode.dev, `code serve-web`) serves webviews from `https://<id>.vscode-cdn.net`; the desktop app uses `vscode-webview://<id>`.
- The list lives in the page, not the URL, so a framing page cannot add itself. Without the attribute nothing changes.
- A frame can't read another origin's location, so the bridge takes the parent's origin from `location.ancestorOrigins` where the browser has it, and otherwise from `document.referrer`. It then posts only to that origin, and it hears only messages from that origin and from the parent window. A wrong guess therefore makes the browser drop the messages; it never sends them to another page. When the browser reports no parent origin (no `ancestorOrigins` and no referrer, for example an `<iframe referrerpolicy="no-referrer">`), the app runs as if unframed.
- A wrong guess is possible where the bridge has to use the referrer: for example, a page that navigates the frame from a listed page. The app then believes it is hosted while its messages go nowhere, and a `save` without an `id` is lost without an error. A host on another origin should send `host` right after `ready`, and an app should send its saves with an `id` and treat a save that gets no `saved` as failed.
- A listed origin gets the same messages a same-origin host gets: it can open documents in the app, and it receives the app's saves. List only origins you trust with the user's documents.
- Keep the list narrow. On a shared development host such as Coder or code-server, `https://*.code.example.com` also covers every port and every dev server that any user runs there, and `vscode-webview://*` covers the webviews of every installed extension. Prefer the narrowest pattern that fits, such as the editor's own address or one user's workspace domain.

Compatibility with the bridges shipped before this crate:

- Fields marked `?` are optional. A host answers `saved` only for a `save` that carries an `id`; an app that sends none treats its save as done once it is posted.
- A host that never sends `host` gets the behaviour of the earlier bridges.
- `size` beside `file` is for hosts and older bridges; this crate takes a `Blob`'s length from the `Blob` itself.
- A page host may dispatch an event while the app's handler runs (a `saved` answering the `save` it just posted). The bridge holds it and calls the handler again once the running call returns, so messages arrive one at a time and in order.
- `version` is `"1"`. The earlier bridges put the app's own release in it (`"0.14.0"`), so a host treats a `version` it doesn't know as `"1"`. The crate sends the release as `appVersion`.

## Use in an app's web shell

```rust
#[cfg(target_arch = "wasm32")]
if let Some(host) = craft_web_host::web::Host::connect("pdfcraft", &canvas) {
    host.listen(move |message| match message {
        craft_web_host::web::Message::Open { name, data } => {
            // Check data.size() against the app's limit before copying it in.
        }
        craft_web_host::web::Message::Saved { id } => { /* clear dirty for that save */ }
        _ => {}
    });
    let _ = host.post_ready(env!("CARGO_PKG_VERSION"), false);
    // Saves: host.post_save(name, &bytes, None, save_as); unsaved state: host.post_dirty(dirty).
}
```

The page command API is [`craft-web-api`](../web-api/README.md) (storytold/craft-libs#413): `window.<app>.request` and the command functions. It sits beside these messages. Opening, saving, the document name and the dirty flag stay here.
