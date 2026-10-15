/**
 * OPFS flush helper smoke (Node 18+). No browser OPFS -- counts pending only.
 * Covers: warm-tier key segments, empty flush, no-OPFS fallback, craftOpfsWarmRoot.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, join } from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const flushPath = join(root, 'assets/opfs-flush.js');

readFileSync(flushPath, 'utf8');
await import(pathToFileURL(flushPath).href);

// -- Hook existence --
const hook = globalThis.craftRuntimeFlushPending;
assert.equal(typeof hook, 'function', 'craftRuntimeFlushPending must be a function');

// -- Key-to-segments: basic media key --
const segments = globalThis.craftOpfsKeyToSegments('files/clip.mp4');
assert.deepEqual(segments, ['effectcraft-store/v1', 'files', 'clip.mp4']);

// -- Key-to-segments: config key --
const configSeg = globalThis.craftOpfsKeyToSegments('config/prefs.json');
assert.deepEqual(configSeg, ['effectcraft-store/v1', 'config', 'prefs.json']);

// -- Key-to-segments: cache key --
const cacheSeg = globalThis.craftOpfsKeyToSegments('cache/slice/abcd/0+512');
assert.deepEqual(cacheSeg, ['effectcraft-store/v1', 'cache', 'slice', 'abcd', '0+512']);

// -- Key-to-segments: render key --
const renderSeg = globalThis.craftOpfsKeyToSegments('render/frame-001.png');
assert.deepEqual(renderSeg, ['effectcraft-store/v1', 'render', 'frame-001.png']);

// -- Key-to-segments: leading slash stripped --
const slashSeg = globalThis.craftOpfsKeyToSegments('/files/audio.wav');
assert.deepEqual(slashSeg, ['effectcraft-store/v1', 'files', 'audio.wav']);

// -- Warm root constant --
assert.equal(globalThis.craftOpfsWarmRoot, 'effectcraft-store/v1');

// -- Flush with one entry (no OPFS in Node -> pending_count returned) --
const out = await hook({
  pending: [{ key: 'config/prefs.json', bytes: [123, 125] }],
});
assert.equal(out.pending_count, 1);

// -- Flush with empty pending --
const emptyOut = await hook({ pending: [] });
assert.equal(emptyOut.pending_count, 0);

// -- Flush with no pending key -> count still reflects list length --
const multiOut = await hook({
  pending: [
    { key: 'files/a.mp4', bytes: [1, 2, 3] },
    { key: 'files/b.mp4', bytes: [4, 5, 6] },
    { key: 'cache/slice/xx/0+10', bytes: new Uint8Array(10) },
  ],
});
assert.equal(multiOut.pending_count, 3);

// -- Flush with missing data field -> treated as empty --
const noDataOut = await hook({});
assert.equal(noDataOut.pending_count, 0);

// -- Flush with null data -> treated as empty --
const nullOut = await hook(null);
assert.equal(nullOut.pending_count, 0);

console.log('opfs-flush.mjs OK');
