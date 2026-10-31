/**
 * The web-component face of the Craft app host: `<craft-app app src>`.
 *
 * Same logic as `mountCraftApp` (this file only wraps it — no logic is written twice) in a custom
 * element: shadow root open, no light-DOM children ever, methods on the element once ready.
 */
import { mountCraftApp } from './core.js';

/** @type {string} The attribute that carries the app's global name. */
const APP = 'app';
/** @type {string} The attribute that carries the iframe's src. */
const SRC = 'src';
/** Events this element re-dispatches, bubbled and composed so they leave the shadow root. */
const EVENTS = { ready: 'craft-ready', failed: 'craft-failed', dirty: 'craft-dirty', save: 'craft-save' };

/**
 * Define `<craft-app>`. Calling it twice with the same tag is safe (the first definition wins).
 *
 * @param {string} [tag='craft-app']
 * @returns {typeof CraftAppElement}
 */
export function defineCraftApp(tag = 'craft-app') {
  if (!customElements.get(tag)) customElements.define(tag, CraftAppElement);
  return CraftAppElement;
}

// A server render (React SSR, TanStack Start) imports this module where no DOM exists: the class
// must still evaluate there. It is only defined as a custom element in a browser.
const ElementBase = globalThis.HTMLElement ?? class {};

export class CraftAppElement extends ElementBase {
  static get observedAttributes() {
    return [APP, SRC];
  }

  /** @type {import('./core.js').CraftAppHandle|null} */
  #handle = null;
  /** @type {Promise<import('./core.js').CraftAppHandle>|null} */
  #mounting = null;
  /** @type {Array<() => void>} */
  #offs = [];

  constructor() {
    super();
    /** @type {ShadowRoot} */
    this.attachShadow({ mode: 'open' });
    const style = document.createElement('style');
    style.textContent = ':host{display:block;position:relative} craft-app-frame,div{display:block;width:100%;height:100%;border:none}';
    /** @type {HTMLDivElement} */
    this.#container = document.createElement('div');
    this.shadowRoot.append(style, this.#container);
  }

  /** @type {HTMLDivElement} */
  #container;

  /** @type {Object|null} */
  get storage() { return this.#storage; }
  set storage(value) { this.#storage = value; this.#applySeams(); }
  #storage = null;

  connectedCallback() {
    this.#connected = true;
    // Safe to repeat: mount only when no app is mounted or mounting. A repeated connect (the
    // element coming back after a move) must not restart a running app.
    this.mount();
  }

  #connected = false;

  disconnectedCallback() {
    // Leaving the DOM never tears the app down: a move can take longer than a microtask, and
    // tearing down on disconnect would kill a running editor with unsaved work. This only
    // records the state; teardown happens only through `destroy()`.
    this.#connected = false;
  }

  attributeChangedCallback(name, oldValue, newValue) {
    if (oldValue === newValue) return;
    if (name !== APP && name !== SRC) return;
    // Without an app or a src there is nothing to show: removing either ends the running app.
    if (newValue === null) this.#teardown();
    else this.#mount();
  }

  /** Mount (or re-mount after an attribute change). */
  #mount() {
    const app = this.getAttribute(APP);
    const src = this.getAttribute(SRC);
    if (!app || !src) return; // wait for both attributes
    if (this.#handle?.app === app && this.#frameSrc === src) return;
    this.#frameSrc = src;
    this.#teardown();
    this.#abort = new AbortController();
    const mounting = mountCraftApp(this.#container, {
      app,
      src,
      storage: this.#storage,
      signal: this.#abort.signal,
    });
    this.#mounting = mounting;
    mounting.then(
      (handle) => {
        // A mount replaced by a newer one, or ended by destroy(), must not come back.
        if (this.#mounting !== mounting) {
          handle.destroy();
          return;
        }
        this.#handle = handle;
        this.#applySeams(); // storage set while the mount was in flight
        for (const [event, name] of Object.entries(EVENTS)) {
          this.#offs.push(handle.on(event, (detail) => this.#dispatch(name, detail)));
        }
        this.#dispatch(EVENTS.ready, {});
      },
      (error) => {
        if (this.#mounting !== mounting) return;
        this.#mounting = null; // a later mount() or connect can try again
        this.#dispatch(EVENTS.failed, { error: error?.message ?? String(error) });
      },
    );
  }

  /** Mount when no app is mounted or mounting; safe to call any number of times. The element
   * mounts itself on connect; this is for a caller that destroyed it and wants it back (React's
   * effect, which runs twice in StrictMode). */
  mount() {
    if (this.#mounting || this.#handle) return;
    this.#mount();
  }

  #frameSrc = null;

  /** Hand a late-arriving storage property to the mounted app's frame. */
  #applySeams() {
    const frame = this.#handle?.frame;
    if (!frame?.contentWindow) return;
    if (this.#storage) frame.contentWindow.craftStorage = this.#storage;
  }

  #dispatch(name, detail) {
    this.dispatchEvent(new CustomEvent(name, { detail, bubbles: true, composed: true }));
  }

  /** Cancels the mount in flight, which removes its frame at once. */
  #abort = null;

  #teardown() {
    this.#abort?.abort();
    this.#abort = null;
    for (const off of this.#offs) off();
    this.#offs = [];
    this.#handle?.destroy();
    this.#handle = null;
    this.#mounting = null;
  }

  /**
   * The core's methods, available on the element once ready; before ready they wait for the mount.
   * @param {string} name
   */
  #forward(name) {
    return (...args) =>
      (this.#mounting ?? Promise.reject(new Error('<craft-app> is not mounted (no app/src)'))).then((handle) =>
        handle[name](...args),
      );
  }

  /** @type {(method: string, params?: Object) => Promise<Object>} */
  request = this.#forward('request');
  /** @type {(command: string, params?: Object) => Promise<Object>} */
  execute = this.#forward('execute');
  /** @type {() => Promise<Object>} */
  commands = this.#forward('commands');
  /** @type {(params?: Object) => Promise<Object>} */
  inspect = this.#forward('inspect');
  /** @type {(params?: Object) => Promise<Object>} */
  screenshot = this.#forward('screenshot');
  /** @type {(file: Blob|ArrayBuffer|Uint8Array, name?: string) => Promise<void>} */
  open = this.#forward('open');
  /** @type {(id: string|number) => Promise<void>} */
  saved = this.#forward('saved');

  /** Tear the mounted app down and empty the element: the element's listeners are unsubscribed
   * and the handle destroyed, which removes the frame. Safe to call twice. After `destroy()`,
   * connecting the element again mounts a fresh app. */
  destroy() {
    this.#teardown();
  }

  /** Whether the app is mounted and answering. */
  get ready() {
    return this.#handle !== null;
  }
}
