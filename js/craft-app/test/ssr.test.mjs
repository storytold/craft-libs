/**
 * A server render imports the element module where there is no DOM (no `HTMLElement`, no
 * `customElements`): the import must not throw. Runs in its own process under `node --test`.
 */
import test from 'node:test';
import assert from 'node:assert/strict';

test('element.js imports without a DOM, as a server render does', async () => {
  assert.equal(globalThis.HTMLElement, undefined, 'this process has no DOM');
  const { CraftAppElement, defineCraftApp } = await import('../element.js');
  assert.equal(typeof CraftAppElement, 'function');
  assert.equal(typeof defineCraftApp, 'function');
});
