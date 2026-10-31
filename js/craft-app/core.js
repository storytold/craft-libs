/**
 * The one implementation of "how a page embeds a Craft app's web build and talks to it".
 *
 * Three faces share this module with no logic written twice:
 * - `mountCraftApp` here (vanilla),
 * - `<craft-app>` in `element.js` (a web component around this),
 * - `CraftApp` in `react.js` (a React wrapper around the element).
 *
 * The framed app exposes `window.<app>` with a `request(method, params)` function and, where its
 * control channel has them, `execute`, `commands`, `docInspect` and `screenshot` — the shape the
 * Rust crate `craft-web-api` installs. The handle calls those functions, so the app's own method
 * names (its `Methods` table) decide what is sent.
 *
 * Documents travel over the app's host messages instead (`src` must carry `?host=parent`): the app
 * posts `<app>:ready`, `<app>:dirty`, `<app>:save` and `<app>:failed` to this window, and the handle
 * posts `<app>:open` and `<app>:saved` to the frame, all same-origin.
 *
 * No build step, no dependencies, any realm: every value that comes from the frame is awaited with
 * `Promise.resolve(...)` and never `instanceof`-checked, because a page in a parent frame passes
 * objects from another realm and `instanceof` fails for them.
 */

/**
 * @typedef {Object} MountOptions
 * @property {string} app The global name the app installs (`"vectorcraft"` → `window.vectorcraft`).
 * @property {string} src The app's URL for the iframe.
 * @property {Object|null} [storage] The page's `craftStorage`, handed to the frame.
 * @property {number} [timeout=180000] Milliseconds to wait for the app to appear in the frame.
 * @property {Document} [document] The document to create the iframe in (tests inject a fake).
 * @property {AbortSignal} [signal] Aborting it while the app is still loading removes the frame and
 *   rejects the mount with the signal's reason.
 */

/**
 * @typedef {Object} CraftAppHandle
 * @property {string} app
 * @property {HTMLIFrameElement} frame
 * @property {(method: string, params?: Object) => Promise<Object>} request
 * @property {(command: string, params?: Object) => Promise<Object>} execute
 * @property {() => Promise<Object>} commands
 * @property {(params?: Object) => Promise<Object>} inspect
 * @property {(params?: Object) => Promise<Object>} screenshot
 * @property {(file: Blob|ArrayBuffer|Uint8Array, name?: string) => Promise<void>} open Open a document
 *   in the app, as File › Open would. Waits for the app's `ready` message.
 * @property {(id: string|number) => void} saved Confirm that the app's `save` with this id is stored.
 * @property {(event: string, fn: (detail: Object) => void) => () => void} on Subscribe; returns `off`.
 * @property {() => void} destroy
 */

const APP_POLL_MS = 100;

/**
 * Mount a Craft app's web build in a same-origin iframe and return its command API.
 *
 * @param {HTMLElement} container Where the iframe is appended.
 * @param {MountOptions} options
 * @returns {Promise<CraftAppHandle>} Resolves with the handle once the app answered; rejects
 *   through the `failed` event path when the app does not appear within `timeout`.
 */
export async function mountCraftApp(container, { app, src, storage = null, timeout = 180000, document: doc = globalThis.document, signal }) {
  const frame = doc.createElement('iframe');
  frame.src = src;
  frame.title = app;
  frame.style.width = '100%';
  frame.style.height = '100%';
  frame.style.border = 'none';
  container.appendChild(frame);

  const listeners = new Map();
  let destroyed = false;
  /** @type {Array<() => void>} Unsubscribers of the frame's forwarded events, for `destroy`. */
  const offs = [];
  const emit = (event, detail) => {
    for (const fn of listeners.get(event) ?? []) {
      try { fn(detail); } catch { /* a listener must not break the others */ }
    }
  };

  const call = (name, ...args) => {
    if (destroyed) return Promise.reject(new Error(`${app} handle was destroyed`));
    const fn = frame.contentWindow?.[app]?.[name];
    if (typeof fn !== 'function') {
      return Promise.reject(new Error(`${app} has no ${name} (yet)`));
    }
    // Any realm: never assume the frame's promise is ours.
    return Promise.resolve(fn.apply(frame.contentWindow[app], args));
  };

  // The app's host messages come to this window from the frame. `ready` can arrive before the
  // app's global does, so the listener is on from the start.
  const win = doc.defaultView ?? globalThis;
  let hostReady;
  const ready = new Promise((resolve) => { hostReady = resolve; });
  // The open() calls still waiting for `ready`: destroy() ends each at once. Each removes itself
  // when its wait settles, so a long-lived handle keeps nothing from opens that finished.
  const waiting = new Set();
  const onMessage = (e) => {
    if (e.source !== frame.contentWindow) return;
    const data = e.data;
    if (!data || typeof data.type !== 'string' || !data.type.startsWith(`${app}:`)) return;
    const kind = data.type.slice(app.length + 1);
    if (kind === 'ready') hostReady(data);
    else if (kind === 'dirty' || kind === 'save' || kind === 'failed') emit(kind, data);
  };
  win.addEventListener?.('message', onMessage);
  offs.push(() => win.removeEventListener?.('message', onMessage));
  /** Post one host message to the frame; same-origin, so the target is this page's origin. */
  const post = (type, fields) => {
    if (destroyed) throw new Error(`${app} handle was destroyed`);
    frame.contentWindow.postMessage({ type: `${app}:${type}`, ...fields }, win.location.origin);
  };

  const handle = /** @type {CraftAppHandle} */ ({
    app,
    frame,
    // Every call goes to the frame's `window.<app>` function of the same purpose and is awaited
    // with `Promise.resolve(...)` so a foreign realm's thenable is awaited too.
    request: (method, params) => call('request', method, params ?? {}),
    execute: (command, params) => call('execute', command, params ?? {}),
    commands: () => call('commands'),
    inspect: (params) => call('docInspect', params ?? {}),
    screenshot: (params) => call('screenshot', params ?? {}),
    open: async (file, name) => {
      let timer;
      let stop;
      const info = await Promise.race([
        ready,
        new Promise((_, reject) => {
          stop = reject;
          waiting.add(stop);
          timer = setTimeout(() => reject(new Error(`${app} did not say it is ready for a document`)), timeout);
        }),
      ]).finally(() => { clearTimeout(timer); waiting.delete(stop); });
      const blob = typeof file?.arrayBuffer === 'function' && typeof file?.size === 'number';
      name = name ?? file?.name ?? 'document';
      if (blob && info.file === true) {
        post('open', { name, file, size: file.size }); // the app reads it in slices
        return;
      }
      // An app without the `file` form takes bytes (and its size cap applies).
      const bytes = blob ? await file.arrayBuffer() : file;
      post('open', { name, bytes });
    },
    saved: (id) => post('saved', { id }),
    on: (event, fn) => {
      const list = listeners.get(event) ?? [];
      list.push(fn);
      listeners.set(event, list);
      return () => {
        const current = listeners.get(event) ?? [];
        const i = current.indexOf(fn);
        if (i >= 0) current.splice(i, 1);
      };
    },
    destroy: () => {
      destroyed = true;
      for (const stop of waiting) stop(new Error(`${app} handle was destroyed`));
      waiting.clear();
      for (const off of offs) off();
      listeners.clear();
      frame.remove();
    },
  });

  // Wait until the app's global exists with a `request` function. The caller gets no handle when
  // this fails or is aborted, so nothing else could remove the frame: every way out removes it.
  const giveUp = (error) => {
    destroyed = true;
    for (const off of offs) off();
    frame.remove();
    throw error;
  };
  const onAbort = () => {
    destroyed = true;
    for (const off of offs) off();
    frame.remove();
  };
  signal?.addEventListener('abort', onAbort, { once: true });
  const deadline = Date.now() + timeout;
  // The page's storage goes to the frame's window as soon as it holds the app's page (not the
  // initial about:blank), on every poll, so it is usually there before the app starts. An app that
  // reads `craftStorage` only once at start-up can still miss it, so it should look again.
  const handOverStorage = () => {
    const w = frame.contentWindow;
    if (storage && w && w.location?.href !== 'about:blank' && w.craftStorage !== storage) w.craftStorage = storage;
  };
  let window_app = null;
  try {
    while (Date.now() < deadline) {
      if (signal?.aborted) break;
      handOverStorage();
      window_app = frame.contentWindow?.[app] ?? null;
      if (window_app && typeof window_app.request === 'function') break;
      await new Promise((resolve) => setTimeout(resolve, APP_POLL_MS));
      window_app = null;
    }
  } catch (error) {
    signal?.removeEventListener('abort', onAbort);
    giveUp(error); // reading the frame threw
  }
  signal?.removeEventListener('abort', onAbort);
  if (signal?.aborted) giveUp(signal.reason ?? new Error(`${app} mount was aborted`));
  if (!window_app || typeof window_app.request !== 'function') {
    const error = `${app} did not appear in the frame within ${Math.round(timeout / 1000)} s`;
    emit('failed', { error });
    giveUp(new Error(error));
  }

  handOverStorage();

  emit('ready', {});
  return handle;
}
