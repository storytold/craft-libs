/**
 * The Craft app host: embed a Craft app's web build in a page and talk to it.
 * Hand-written types for `core.js`, the root entry (plain ES modules, no build step).
 * `element.d.ts` and `react.d.ts` cover the `./element` and `./react` entries.
 */
export as namespace CraftAppHost;

/** What a call resolves with: the method's `result`. A failed call rejects with an `Error` carrying the app's `error`. */
export type AppResult = unknown;

export interface MountOptions {
  /** The global name the app installs (`"vectorcraft"` → `window.vectorcraft`). */
  app: string;
  /** The app's URL for the iframe. */
  src: string;
  /** The page's `craftStorage`, handed to the frame. */
  storage?: unknown | null;
  /** Milliseconds to wait for the app to appear in the frame. Default 180000. */
  timeout?: number;
  /** The document to create the iframe in (tests inject a fake). */
  document?: Document;
  /** Aborting it while the app is still loading removes the frame and rejects the mount. */
  signal?: AbortSignal;
}

export interface CraftAppHandle {
  readonly app: string;
  readonly frame: HTMLIFrameElement;
  request(method: string, params?: object): Promise<AppResult>;
  execute(command: string, params?: object): Promise<AppResult>;
  commands(): Promise<AppResult>;
  inspect(params?: object): Promise<AppResult>;
  screenshot(params?: object): Promise<AppResult>;
  /** Open a document in the app, as File › Open would, once the app's `ready` message has come.
   * A `File`/`Blob` goes as it is when the app's `ready` says `file: true` (the app reads it in
   * slices); otherwise it is read to bytes and the app's size cap applies. */
  open(file: Blob | ArrayBuffer | Uint8Array, name?: string): Promise<void>;
  /** Confirm that the app's `save` with this `id` is stored; until then the app stays dirty. */
  saved(id: string | number): void;
  /** Subscribe to `dirty`, `save` (`{id, name, bytes}`: the app handing a file over; answer with
   * `saved(id)` once it is stored) or `failed` (`{error}`, or `{id, error}` for an unconfirmed save);
   * returns the unsubscribe function. A failed mount rejects `mountCraftApp` instead. */
  on(event: 'dirty' | 'save' | 'failed', fn: (detail: Record<string, unknown>) => void): () => void;
  /** Remove the frame and every listener. */
  destroy(): void;
}

/** Vanilla face: mount a Craft app's web build and get its command API. */
export function mountCraftApp(container: HTMLElement, options: MountOptions): Promise<CraftAppHandle>;
