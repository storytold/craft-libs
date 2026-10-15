/**
 * FS Access cold-tier host smoke (Node 18+).
 * Stubs FileSystemDirectoryHandle to verify path routing and write/read round trips.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, join } from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const hostPath = join(root, 'assets/fs-access-host.js');

readFileSync(hostPath, 'utf8');
await import(pathToFileURL(hostPath).href);

// -- Globals exported --
assert.equal(typeof globalThis.craftFsAccessKeySegments, 'function');
assert.equal(typeof globalThis.craftFsAccessWriteKey, 'function');
assert.equal(typeof globalThis.craftFsAccessReadKey, 'function');
assert.equal(typeof globalThis.craftFsAccessFlushPending, 'function');
assert.equal(typeof globalThis.craftFsAccessMountVolume, 'function');
assert.equal(typeof globalThis.craftFsAccessUnmountVolume, 'function');
assert.equal(globalThis.craftFsAccessVolumesPrefix, 'volumes');

// -- Path segments match DefaultTierCoordinator::cold_relative_path --
const volId = 'cold-root-7';
const mediaSeg = globalThis.craftFsAccessKeySegments(volId, 'files/clip.mp4');
assert.deepEqual(mediaSeg, ['volumes', 'cold-root-7', 'files', 'clip.mp4']);

const cacheSeg = globalThis.craftFsAccessKeySegments(volId, 'cache/slice/abcd/0+512');
assert.deepEqual(cacheSeg, ['volumes', 'cold-root-7', 'cache', 'slice', 'abcd', '0+512']);

// -- Config keys are excluded from cold storage (empty array) --
const configSeg = globalThis.craftFsAccessKeySegments(volId, 'config/prefs.json');
assert.deepEqual(configSeg, [], 'config keys must not map to cold paths');

// -- Leading slash stripped --
const slashSeg = globalThis.craftFsAccessKeySegments(volId, '/files/audio.wav');
assert.deepEqual(slashSeg, ['volumes', 'cold-root-7', 'files', 'audio.wav']);

// -- Stub FileSystemDirectoryHandle for write/read round-trip --
// In-memory tree: Map<name, { isDir: boolean, children: Map|null, data: Uint8Array|null }>
function makeNode(isDir = true) {
  return { isDir, children: isDir ? new Map() : null, data: null };
}

function makeStubHandle(node) {
  return {
    _node: node,
    async getDirectoryHandle(name, opts = {}) {
      if (!node.children.has(name)) {
        if (!opts.create) throw new Error(`NotFoundError: ${name}`);
        node.children.set(name, makeNode(true));
      }
      const child = node.children.get(name);
      if (!child.isDir) throw new Error(`TypeMismatchError: ${name}`);
      return makeStubHandle(child);
    },
    async getFileHandle(name, opts = {}) {
      if (!node.children.has(name)) {
        if (!opts.create) throw new Error(`NotFoundError: ${name}`);
        node.children.set(name, makeNode(false));
      }
      const child = node.children.get(name);
      if (child.isDir) throw new Error(`TypeMismatchError: ${name}`);
      return {
        async createWritable() {
          let buf = null;
          return {
            async write(data) { buf = data; },
            async close() { child.data = buf; },
          };
        },
        async getFile() {
          return {
            async arrayBuffer() {
              return child.data instanceof Uint8Array
                ? child.data.buffer.slice(
                    child.data.byteOffset,
                    child.data.byteOffset + child.data.byteLength,
                  )
                : new ArrayBuffer(0);
            },
          };
        },
      };
    },
  };
}

// Make FileSystemDirectoryHandle available so hasFsAccess() returns true.
globalThis.FileSystemDirectoryHandle = class {};

const rootNode = makeNode(true);
const rootHandle = makeStubHandle(rootNode);

// -- Write a media key --
await globalThis.craftFsAccessWriteKey(rootHandle, volId, 'files/clip.mp4', new Uint8Array([1, 2, 3]));
const read = await globalThis.craftFsAccessReadKey(rootHandle, volId, 'files/clip.mp4');
assert.ok(read instanceof Uint8Array, 'read back Uint8Array');
assert.deepEqual(Array.from(read), [1, 2, 3]);

// -- Config key write is silently skipped --
await globalThis.craftFsAccessWriteKey(rootHandle, volId, 'config/prefs.json', new Uint8Array([99]));
const configRead = await globalThis.craftFsAccessReadKey(rootHandle, volId, 'config/prefs.json');
assert.equal(configRead, null, 'config keys must not be stored cold');

// -- Read missing key returns null --
const missing = await globalThis.craftFsAccessReadKey(rootHandle, volId, 'files/no-such.mp4');
assert.equal(missing, null);

// -- flushPending batch --
const flushResult = await globalThis.craftFsAccessFlushPending(
  volId,
  [
    { key: 'files/a.mp4', bytes: [10, 20] },
    { key: 'cache/slice/ff/0+2', bytes: [30, 40] },
    { key: 'config/skip.json', bytes: [99] },
  ],
  rootHandle,
);
assert.equal(flushResult.pending_count, 3);

const aRead = await globalThis.craftFsAccessReadKey(rootHandle, volId, 'files/a.mp4');
assert.deepEqual(Array.from(aRead), [10, 20]);

const cacheRead = await globalThis.craftFsAccessReadKey(rootHandle, volId, 'cache/slice/ff/0+2');
assert.deepEqual(Array.from(cacheRead), [30, 40]);

// -- Mount/unmount volume registry --
globalThis.craftFsAccessMountVolume(volId, rootHandle);
const flushViaRegistry = await globalThis.craftFsAccessFlushPending(
  volId,
  [{ key: 'files/reg.mp4', bytes: [55, 66] }],
);
assert.equal(flushViaRegistry.pending_count, 1);

globalThis.craftFsAccessUnmountVolume(volId);
// After unmount, no handle -> pending_count still returns length (no-op flush).
const afterUnmount = await globalThis.craftFsAccessFlushPending(
  volId,
  [{ key: 'files/reg.mp4', bytes: [55, 66] }],
);
assert.equal(afterUnmount.pending_count, 1);

console.log('fs-access-host.mjs OK');
