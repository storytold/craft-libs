# craft-web-host

The host messages between a Craft app's web build and the page that hosts it, as one contract and one bridge for every app (storytold/craft-libs#358). A page frames the app with `?host=parent`, or runs it directly with `data-host="page"` on its canvas; either way the same messages pass files in, saves out, and unsaved-changes state, so a page can host any of the apps the same way.

The crate has three parts:

- **The contract** (all targets): the message kinds and their `<app>:<kind>` types, the contract version, how the hosting mode is read from the URL and the canvas, and the clean-up of file names a host passes in. Pure, tested natively.
- **The statuses** (all targets): `status::HostStatus`, what the title bar says about the document — saved, saving, unsaved, the stored version, how many people are in it, read-only — built from the same messages, with the priority rules in one place. Plain data, tested natively; the app supplies the words.
- **The bridge** (`web`, wasm32 only): `Host::connect(app, canvas)` finds the host, `listen` hears its messages, and `post_ready`, `post_save`, `post_dirty`, `post_failed` and `post_handoff` answer it. Only the same-origin parent window is heard and answered; with a parent on another origin the app runs as if unframed.
- **The file service** (`files`, wasm32 only, deprecated): `BrowserFiles::new(wake, max_read)` wraps `craft_platform::browser_files::BrowserFiles`, which holds its picks and saves (`Files::save`). Only `write` is its own: it posts the bytes to the page through the connection `Host::connect` made, or downloads them when no page hosts the app. New code uses the platform service directly.

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

## Title bar statuses

`status::HostStatus` takes one input per thing the app learns, in any order:

```rust
use craft_web_host::status::{HostStatus, SaveState, StatusInput};

let mut status = HostStatus::new();
status.apply(StatusInput::Edited); // a local change
status.apply(StatusInput::SavePosted { id: Some("7") }); // the `save` it just posted
status.apply(StatusInput::Saved { id: "7" }); // the host's `saved`
assert_eq!(status.save_state(), SaveState::Saved);

// Each frame, the parts the title bar draws, in the order it draws them.
for part in status.cluster().parts() {
    // `ClusterPart::{Headline, Participants, Sync, Version}`, worded by the app's language pack.
    let _ = part;
}
```

`cluster().parts()` leaves out what does not apply: no participant count without a session, no sync word while `synced`, no version before a commit. `cluster().error` is the reason a save was refused, for as long as the document shows as unsaved.

## One authority per document

Three of the inputs above already have an owner elsewhere, and the status is only a view of
whichever one applies. The rule is the document's origin, one input per document:

- **A document opened from a cloud entry** is settled by the cloud contract
  (storytold/craft-libs#160). `craft_ui::cloud::Event::Saved` answers the save job with the entry
  it produced or the `craft_ui::cloud::Fault` that refused it, and a stale etag is
  `Fault::Conflict`. The app tracks the revision each save in flight encoded, so a save that lands
  marks exactly that revision saved. It does not apply the host page's `saved`/`failed` to this
  document: the cloud contract wins, and an app hosted in a page and signed in to a provider shows
  the conflict the cloud pages hold rather than the page's success.
- **A document the page handed in** has no cloud entry, so its save state is the host page's
  `saved`/`failed`, which is what the inputs above derive.

Read-only follows the same split: `craft_cloud::Entry::writable` for a cloud entry,
`StatusInput::ReadOnly` for a session whose role is `viewer` (or a host-controlled read-only
session, storytold/craft-libs#609). The bar takes one of them per document, by the same rule.

`cluster().participants` is the count of people in the hosted page's session, as the app posted it
in `session-state`/`presence`. It is not RFC-0020 item 2's presence count: that contract, when its
own PR lands beside `craft-cloud`, owns presence fan-out, session state and the sync word.

`craft-web-host` is the contract between an app's web build and the page that hosts it;
`craft-worker-host` (storytold/craft-libs#662) is the protocol between a tab and its
`SharedWorker`. Different concerns, names one word apart.
