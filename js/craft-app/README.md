# @craft/app

How a page embeds a Craft app's web build and talks to it: one implementation, three faces — a
vanilla `mountCraftApp`, a `<craft-app>` web component and a React `<CraftApp>` wrapper.

Plain ES modules, no build step, no dependencies (React is an optional peer for the React face).

## The three faces

### Vanilla (`mountCraftApp`)

```js
import { mountCraftApp } from '@craft/app';

const app = await mountCraftApp(document.getElementById('host'), {
  app: 'vectorcraft', src: '/apps/vectorcraft/?host=parent',
  storage: myStorage,
});
await app.execute('object.rectangle', { width: 100 });
app.on('dirty', () => setUnsaved(true));
```

### Web component (`<craft-app>`)

```js
import { defineCraftApp } from '@craft/app/element';
defineCraftApp();
```

```html
<craft-app app="vectorcraft" src="/apps/vectorcraft/?host=parent"></craft-app>
```

```js
const el = document.querySelector('craft-app');
el.storage = myStorage; // property, not attribute
el.addEventListener('craft-ready', () => el.execute('object.rectangle', { width: 100 }));
```

### React (`<CraftApp>`)

```jsx
import { CraftApp } from '@craft/app/react';

<CraftApp app="vectorcraft" src="/apps/vectorcraft/?host=parent"
  storage={myStorage}
  onReady={() => setReady(true)} onDirty={() => setUnsaved(true)} />;
```

`storage` is set as an element property (objects never go through attributes),
and the events are subscribed with `addEventListener` in an effect — JSX event props on custom
elements are unreliable after hydration.

The storage becomes the frame's `window.craftStorage` as soon as the frame holds the app's page, and is checked again every 100 ms until the app answers, so it is usually there before the app starts. It is not guaranteed to be there before the app's first code runs: an app that reads `craftStorage` only once at start-up should look again later.

## Documents and events

The framed build must be loaded with `?host=parent` in `src`: documents and the app's signals then travel as the app's host messages, same-origin `postMessage` between this page and the frame.

- `open(file, name?)` opens a document as File › Open would, once the app has posted `<app>:ready`. A `File` or `Blob` goes to the app as it is when its `ready` says `file: true` (the app reads it in slices); otherwise it is read to bytes and the app's size cap applies. `ArrayBuffer` and `Uint8Array` go as bytes.
- `save` (`{id, name, bytes}`) is the app handing a file over: Save, Save As, Export. Store it, then call `saved(id)`; until then the app stays dirty, and after 30 s it reports `failed` with that `id`.
- `dirty` (`{dirty}`) whenever unsaved changes appear or go away; `failed` (`{error}`) when the app could not take a document or a save went unconfirmed.

On the handle these are `on('dirty' | 'save' | 'failed', fn)`; `mountCraftApp` itself resolves once the app is ready and rejects when it is not. On the element they are `craft-ready`, `craft-failed`, `craft-dirty` and `craft-save` (bubbled, composed), and `open` and `saved` are element methods.

## Lifecycle: destroy, not disconnect

When you are done with the app, call `element.destroy()` on `<craft-app>` (or `handle.destroy()` on the vanilla handle) — it unsubscribes the listeners, destroys the handle and removes the frame; calling it twice is safe, and after it, connecting the element again or calling `element.mount()` mounts a fresh app (`mount()` does nothing while an app is mounted or mounting). Leaving the DOM alone never tears the app down: a move can take longer than a microtask, and tearing down on disconnect would kill a running editor with unsaved work, so `disconnectedCallback` only records that the element is disconnected and the same mounted app survives a move. React's `<CraftApp>` calls `mount()` in an effect and `destroy()` in its cleanup, so unmounting the component frees the app, and StrictMode's second run of the effect brings it back.

## Relation to the Rust crate

The framed app exposes `window.<app>` — `request(method, params)` and, where its control channel
has them, `execute`, `commands`, `docInspect` and `screenshot` — the shape the Rust crate
`craft-web-api` installs (`crates/web-api/`). The handle calls those functions, so the app's own
method table decides which control method each one sends. Each resolves with the method's
`result` and rejects with an `Error` carrying the app's `error`.

## Tests

`node --test 'js/craft-app/test/**/*.test.mjs'` — no dependencies, no build step, no browser.
