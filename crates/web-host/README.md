# craft-web-host

The host messages between a Craft app's web build and the page that hosts it, as one contract and one bridge for every app (storytold/craft-libs#358). A page frames the app with `?host=parent`, or runs it directly with `data-host="page"` on its canvas; either way the same messages pass files in, saves out, and unsaved-changes state, so a page can host any of the apps the same way.

The crate has two parts:

- **The contract** (all targets): the message kinds and their `<app>:<kind>` types, the contract version, how the hosting mode is read from the URL and the canvas, and the clean-up of file names a host passes in. Pure, tested natively.
- **The bridge** (`web`, wasm32 only): `Host::connect(app, canvas)` finds the host, `listen` hears its messages, and `post_ready`, `post_save`, `post_dirty`, `post_failed` and `post_handoff` answer it. Only the same-origin parent window is heard and answered; with a parent on another origin the app runs as if unframed.

## Messages

| Kind | Direction | Fields | Meaning |
|---|---|---|---|
| `ready` | app → host | `version`, `appVersion?`, `file?` | the app can take a document; `file: true` means it reads a `File`/`Blob` in slices |
| `host` | host → app | `version`, `handoff?` | what the host supports; `handoff: true` means it routes hand-offs |
| `open` | host → app | `name`, and `bytes` or `file` + `size` | open a document as File › Open would |
| `fonts` | host → app | `indexes` | font indexes the host supplies: each `{bytes}` or `{url, sha256}` |
| `save` | app → host | `name`, `bytes`, `id?`, `saveAs?` | Save, Save As or Export: the host stores the file |
| `saved` | host → app | `id` | the save with that `id` is stored; the app clears dirty |
| `dirty` | app → host | `dirty` | unsaved changes appeared or went away |
| `failed` | app → host | `error`, `id?` | a document or a save could not be taken, or a `fonts` list was refused (no `id`; `error` starts `fonts:`) |
| `handoff` | app → host | `name`, and `bytes` or `file` + `size` | this file belongs to another app |
| `handoff-done` | host → app | `name`, `app` | the host gave it to the app that edits it |

`fonts` (RFC-0034, storytold/craft-libs#2331) hands an app indexes in craft-font-index format; the app fetches a face from one only when type in a document uses it. `indexes` holds 1 to 16 entries, each either `{bytes}` (an `ArrayBuffer` or `Uint8Array`, JSON or gzipped JSON, 1 byte to 32 MiB) or `{url, sha256}`. `undefined` and `null` count as an absent field. The bridge catches honest mistakes only, as it does for `open`, and refuses the whole list when any part is wrong, with one `FontsProblem` for the first fault:

- `NotAList`: `indexes` is missing or not an array. `NoIndexes`: it is empty. `TooMany`: more than 16 entries.
- `BadEntry`: an entry that is not an object, has neither `bytes` nor `url`, has both, is `{bytes, sha256}`, or has `bytes` that is neither an `ArrayBuffer` nor a `Uint8Array` (in page mode, one from the app page's own realm).
- `NoBytes`: `bytes` that are empty, or were transferred away (a detached buffer reads as empty). `TooLarge`: `bytes` over 32 MiB.
- `BadUrl`: a `url` that is not a string, not lowercase `https://` with something after it, over 1024 bytes, or holding a control character, whitespace or `\`. The bridge does not parse the URL; one the browser refuses fails when the app fetches it, which the app reports as `failed`.
- `BadSha256`: a `sha256` that is missing (`{url}` alone), not a string, or not 64 lowercase hex digits. The URL is judged before the sha256.

The host is trusted (RFC-0034 section 7). Its message is plain data, as structured clone gives in parent mode: arrays, objects, strings, `ArrayBuffer`, `Uint8Array`. A page host that passes an object with getters, a `Proxy`, a subclass or a view of the app's own memory is outside the contract, and the bridge promises nothing for it. The bridge copies no index's bytes into wasm memory until the app takes them (in parent mode a queued message keeps the browser's structured clone of each whole buffer alive); `url` and `sha256` are (at most 1024 and 64 bytes once read; a `url` is read from a string of at most 1024 UTF-16 units, so from at most 3072 bytes). `Message::Fonts` carries `FontIndexView`s and `FontIndexView::copy` checks the length again (read as a float, so a buffer of 4 GiB or more is refused, not wrapped) and copies through a view of that fixed length, at most 32 MiB per index and 16 times that for a set. An app that ignores `fonts` copies only those two short strings. A refused list still reaches the app, as `Message::Fonts { indexes: Err(problem) }`; `FontsProblem` implements `Display`, and the app reports it with `failed`: no `id` (a `fonts` failure carries none), and an `error` that starts `fonts:`, so a host can tell it from a failed document. An app starts its own `fonts` refusals with `fonts:` too, and its own document errors never start `fonts:`, so a host can tell the two apart. The app keeps the set it had. The bridge does not parse an index: the app checks each one itself (version, URL templates, safe paths, sha256s, sizes) and takes the list whole or not at all. Each `fonts` message replaces the host's set: a host adds an index by sending the whole set again, and an index it leaves out is gone. A host that transfers `bytes` gives the buffer up, so it keeps its own copy if it may send the set again. A host sends `fonts` after `ready`, and again after every `ready`: a reloaded app starts with no host set, and a `fonts` sent before the app listens is lost. An app applies sets in the order they arrive: a later accepted set replaces an earlier one, and a refused one changes nothing. An app that does not handle `fonts` ignores it, so silence tells a host nothing. The host's sending `fonts` is its consent to fetch from those indexes. A document and the app's command channel give no way to send it, but any script able to run in the host page (`data-host="page"`) or in the app's frame (`?host=parent`) can dispatch one. There is no answer message besides `failed`.

With `?host=parent` a message is an object with `type` (`pdfcraft:open`) and the fields, sent with same-origin `postMessage`; `bytes` go as a transferred `ArrayBuffer`. With `data-host="page"` it is a `CustomEvent` of that type on the canvas, with the fields in `detail`.

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
