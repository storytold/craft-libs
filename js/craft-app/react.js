/**
 * The React face of the Craft app host: a `<CraftApp>` component over the `<craft-app>` element.
 *
 * React is a peer dependency, imported with named imports; no JSX, so there is no build step.
 * `storage` is set as an element property in an effect (React drops object props through
 * attributes), and the four events are subscribed with `addEventListener` in an effect — JSX event
 * props on custom elements are unreliable after hydration (see the web-interop contract).
 */
import { createElement, forwardRef, useEffect, useImperativeHandle, useRef } from 'react';
import { defineCraftApp } from './element.js';

/**
 * @typedef {Object} CraftAppProps
 * @property {string} app The app's global name (attribute).
 * @property {string} src The app's URL (attribute).
 * @property {Object|null} [storage] Set as a property.
 * @property {((detail: Object) => void)} [onReady]
 * @property {((detail: Object) => void)} [onFailed]
 * @property {((detail: Object) => void)} [onDirty]
 * @property {((detail: Object) => void)} [onSave]
 * @property {import('react').CSSProperties} [style]
 * @property {string} [className]
 * @property {import('react').Ref} [ref] Forwards the `<craft-app>` element.
 */

/** The element's events and their component props. */
const EVENT_PROPS = [
  ['craft-ready', 'onReady'],
  ['craft-failed', 'onFailed'],
  ['craft-dirty', 'onDirty'],
  ['craft-save', 'onSave'],
];

/**
 * @type {import('react').ForwardRefExoticComponent<import('react').PropsWithoutRef<CraftAppProps> & import('react').RefAttributes<HTMLElement>>}
 */
export const CraftApp = forwardRef(function CraftApp(
  { app, src, storage, onReady, onFailed, onDirty, onSave, style, className },
  ref,
) {
  /** @type {import('react').MutableRefObject<HTMLElement|null>} */
  const innerRef = useRef(null);

  // Define the element client-side only and idempotently; safe under SSR (no window there).
  useEffect(() => {
    if (typeof customElements !== 'undefined') defineCraftApp();
  }, []);

  // Object/function values as properties, never attributes.
  useEffect(() => {
    const el = innerRef.current;
    if (!el) return;
    if (storage !== undefined) el.storage = storage;
  }, [storage]);

  // Custom-named events: addEventListener in an effect, removed on cleanup.
  useEffect(() => {
    const el = innerRef.current;
    if (!el) return;
    const pairs = EVENT_PROPS.map(([event, prop]) => [event, { onReady, onFailed, onDirty, onSave }[prop]]);
    const offs = pairs
      .filter(([, fn]) => fn)
      .map(([event, fn]) => {
        el.addEventListener(event, fn);
        return () => el.removeEventListener(event, fn);
      });
    return () => { for (const off of offs) off(); };
  }, [onReady, onFailed, onDirty, onSave]);

  // Unmounting frees the app: the element's destroy() tears the frame and its listeners down.
  // Disconnecting alone never does (a DOM move must not kill the app), so React must ask for it.
  useEffect(() => {
    const el = innerRef.current;
    if (!el) return;
    el.mount(); // StrictMode runs this twice: the first cleanup destroyed the app
    return () => el.destroy();
  }, []);

  useImperativeHandle(ref, () => innerRef.current);

  return createElement('craft-app', {
    app,
    src,
    style,
    class: className,
    ref: innerRef,
  });
});
