/**
 * OPFS flush helper smoke (Node 18+). No browser OPFS — counts pending only.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, join } from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const flushPath = join(root, 'assets/opfs-flush.js');

readFileSync(flushPath, 'utf8');
await import(pathToFileURL(flushPath).href);

const hook = globalThis.craftRuntimeFlushPending;
assert.equal(typeof hook, 'function');

const out = await hook({
  pending: [{ key: 'config/prefs.json', bytes: [123, 125] }],
});
assert.equal(out.pending_count, 1);

const segments = globalThis.craftOpfsKeyToSegments('files/clip.mp4');
assert.deepEqual(segments, ['effectcraft-store/v1', 'files', 'clip.mp4']);

console.log('opfs-flush.mjs OK');
