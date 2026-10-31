/** Types for `element.js` (`@craft/app/element`). */
import type { AppResult } from './core.js';

/** The web-component face. Defined by `defineCraftApp(tag)`; never define it at import time. */
export class CraftAppElement extends HTMLElement {
  static observedAttributes: string[];
  /** The page's `craftStorage` (a property, never an attribute). */
  storage: unknown | null;
  /** True once the app is mounted and answering. */
  readonly ready: boolean;
  request(method: string, params?: object): Promise<AppResult>;
  execute(command: string, params?: object): Promise<AppResult>;
  commands(): Promise<AppResult>;
  inspect(params?: object): Promise<AppResult>;
  screenshot(params?: object): Promise<AppResult>;
  /** Open a document in the app (see `CraftAppHandle.open`). */
  open(file: Blob | ArrayBuffer | Uint8Array, name?: string): Promise<void>;
  /** Confirm a `craft-save` with this `id` is stored. */
  saved(id: string | number): Promise<void>;
  /** Mount when no app is mounted or mounting; safe to call any number of times (the element
   * also mounts itself on connect). */
  mount(): void;
  /** Tear the mounted app down and empty the element: the element's listeners are unsubscribed
   * and the handle destroyed, which removes the frame. Safe to call twice; after it, connecting
   * the element again mounts a fresh app. */
  destroy(): void;
}

/**
 * Define `<craft-app>` (or another tag). Idempotent; no side effect on import.
 * Events on the element, `bubbles: true, composed: true`:
 * `craft-ready`, `craft-failed` (`{error}`), `craft-dirty`, `craft-save` (`craft-saved` is
 * reserved for a host-confirmed save and is not emitted yet).
 */
export function defineCraftApp(tag?: string): typeof CraftAppElement;
