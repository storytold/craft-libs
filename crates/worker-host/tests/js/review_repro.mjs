// Regression tests for PR #662's web-runtime slice, reproducing the three failures from
// skylartaylor's review (storytold/craft-contrib#39, gist c1108ecf20d63c5e3183947408ddab7a):
//
//   1. assets/opfs-flush.js hard-coded a single, slash-containing OPFS directory name, which
//      Chrome's getDirectoryHandle rejects ("Name is not allowed"), and baked one app's name
//      into a shared asset.
//   2. storage_flushed could be reported without the sink actually persisting anything, and
//      out-of-order flush completions could let stale bytes win.
//
// (Failure 3, unbounded eviction retention, is a Rust-only concern and is covered by
// crates/runtime/tests/review_repro.rs instead.)
//
// Run: node --test crates/worker-host/tests/js/review_repro.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';

const ASSETS = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../../assets');
const readAsset = (name) => fs.readFileSync(path.join(ASSETS, name), 'utf8');

// Boot the dedicated worker. `missing` lists scripts whose importScripts throws (404/blocked).
function boot({ missing = [], flushHook } = {}) {
  const posted = [];
  const ctx = { console, setTimeout, Promise, Uint8Array, ArrayBuffer, Map };
  ctx.self = ctx;
  ctx.globalThis = ctx;
  ctx.postMessage = (m) => posted.push(m);
  ctx.importScripts = (name) => {
    if (missing.includes(name)) throw new Error(`NetworkError: ${name} failed to load`);
    vm.runInContext(readAsset(name), ctx, { filename: name });
  };
  vm.createContext(ctx);
  vm.runInContext(readAsset('craft-dedicated-worker.js'), ctx);
  if (flushHook) ctx.craftRuntimeFlushPending = flushHook;
  const send = (data) => ctx.onmessage({ data });
  return { ctx, posted, send };
}
const tick = () => new Promise((r) => setTimeout(r, 5));

// --- Failure 1: hard-coded, slash-containing OPFS directory -----------------------------------

// Minimal in-memory stand-in for the OPFS directory API, strict about the one rule that matters
// here: a path SEGMENT must never contain '/'. This is exactly what throws
// "TypeError: Failed to execute 'getDirectoryHandle' on 'FileSystemDirectoryHandle': Name is not
// allowed." in real Chrome.
class FakeDir {
  constructor() {
    this.dirs = new Map();
    this.files = new Map();
  }
  async getDirectoryHandle(name, opts) {
    if (typeof name !== 'string' || name.length === 0 || name.includes('/')) {
      throw new TypeError("Failed to execute 'getDirectoryHandle': Name is not allowed.");
    }
    if (!this.dirs.has(name)) {
      if (!(opts && opts.create)) throw new Error('NotFoundError');
      this.dirs.set(name, new FakeDir());
    }
    return this.dirs.get(name);
  }
  async getFileHandle(name, opts) {
    if (typeof name !== 'string' || name.length === 0 || name.includes('/')) {
      throw new TypeError("Failed to execute 'getFileHandle': Name is not allowed.");
    }
    if (!this.files.has(name)) {
      if (!(opts && opts.create)) throw new Error('NotFoundError');
      this.files.set(name, new FakeFile());
    }
    return this.files.get(name);
  }
}
class FakeFile {
  constructor() {
    this.bytes = null;
  }
  async createWritable() {
    const self = this;
    return {
      async write(payload) {
        self.bytes = Uint8Array.from(payload);
      },
      async close() {},
    };
  }
}

function bootOpfs({ appId } = {}) {
  const root = new FakeDir();
  const ctx = { console, setTimeout, Uint8Array, ArrayBuffer };
  ctx.globalThis = ctx;
  ctx.navigator = { storage: { getDirectory: async () => root } };
  if (appId !== undefined) ctx.craftAppId = appId;
  vm.createContext(ctx);
  vm.runInContext(readAsset('opfs-flush.js'), ctx);
  return { ctx, root };
}

test('H-repro: the old single-segment root name is exactly what Chrome rejects', async () => {
  const root = new FakeDir();
  await assert.rejects(
    () => root.getDirectoryHandle('effectcraft-store/v1', { create: true }),
    /Name is not allowed/,
    'a single directory-handle call with an embedded slash must throw, matching the gist repro',
  );
});

test('OPFS: craftRuntimeFlushPending namespaces under the app id, one segment per call', async () => {
  const { ctx, root } = bootOpfs({ appId: 'collab.photo' });
  const result = await ctx.craftRuntimeFlushPending({ pending: [{ key: 'docs/a.bin', bytes: [1, 2, 3] }] });
  // `result` is an object from the vm context's own realm, so compare the field rather than the
  // whole object (deepStrictEqual also checks prototypes, which differ across vm realms).
  assert.equal(result.pending_count, 1);

  // No literal app name anywhere, and every segment was created individually (no slash reached
  // getDirectoryHandle, or FakeDir's own check above would have thrown).
  assert.deepEqual([...root.dirs.keys()], ['collab-photo']);
  const appDir = root.dirs.get('collab-photo');
  assert.deepEqual([...appDir.dirs.keys()], ['store']);
  const storeDir = appDir.dirs.get('store');
  assert.deepEqual([...storeDir.dirs.keys()], ['v1']);
  const v1Dir = storeDir.dirs.get('v1');
  const docsDir = v1Dir.dirs.get('docs');
  assert.ok(docsDir, 'nested key segments still create their own subdirectories');
  assert.deepEqual(Array.from(docsDir.files.get('a.bin').bytes), [1, 2, 3]);
});

test('OPFS: two app ids never collide in the same OPFS tree', async () => {
  const a = bootOpfs({ appId: 'collab.photo' });
  const b = bootOpfs({ appId: 'filmcraft' });
  await a.ctx.craftRuntimeFlushPending({ pending: [{ key: 'config/prefs.json', bytes: [1] }] });
  await b.ctx.craftRuntimeFlushPending({ pending: [{ key: 'config/prefs.json', bytes: [2] }] });
  // Separate roots in this test, but the point is the namespace segment itself: each app gets
  // its own top-level directory name derived from its own app id, never a shared literal.
  assert.deepEqual([...a.root.dirs.keys()], ['collab-photo']);
  assert.deepEqual([...b.root.dirs.keys()], ['filmcraft']);
});

test('OPFS: an app id that is empty, missing, or itself slash-laden still yields one safe segment', async () => {
  for (const appId of [undefined, '', 'nested/app/id', '..', '.']) {
    const { ctx, root } = bootOpfs({ appId });
    // eslint-disable-next-line no-await-in-loop
    await ctx.craftRuntimeFlushPending({ pending: [{ key: 'config/a', bytes: [1] }] });
    const [namespace] = [...root.dirs.keys()];
    assert.ok(namespace && !namespace.includes('/'), `sanitized namespace must be one safe segment, got ${JSON.stringify(namespace)}`);
  }
});

// --- Failure 2: acknowledgment without persistence, and revision tying ------------------------

test('H4: opfs-flush.js fails to load -> storage_flushed reported and the write is gone', async () => {
  const w = boot({ missing: ['opfs-flush.js', 'fs-access-host.js'] });
  w.send({ type: 'storage_put', key: 'doc/1', bytes: [1, 2, 3] });
  w.send({ type: 'storage_flush' });
  await tick();
  w.send({ type: 'storage_flush' }); // a retry finds nothing left to persist, if the bug is present
  await tick();
  const report = JSON.stringify(w.posted);
  assert.ok(
    !(w.posted[0]?.type === 'storage_flushed' && w.posted[0]?.pending_count === 1 && w.posted[1]?.pending_count === 0),
    `nothing persisted doc/1 (no hook), yet worker reported success and dropped it: ${report}`,
  );
  assert.equal(w.posted[0]?.type, 'error', `missing hook must report error, not success: ${report}`);
});

test('H4: a hook that persists 1 of 2 entries -> the other is dropped from the buffer', async () => {
  const persisted = new Map();
  const w = boot({
    missing: ['fs-access-host.js'],
    flushHook: async ({ pending }) => {
      persisted.set(pending[0].key, pending[0].bytes); // quota hit after the first entry
      return { pending_count: 1 };
    },
  });
  w.send({ type: 'storage_put', key: 'a', bytes: [1] });
  w.send({ type: 'storage_put', key: 'b', bytes: [2] });
  w.send({ type: 'storage_flush' });
  await tick();
  let retried = [];
  w.ctx.craftRuntimeFlushPending = async ({ pending }) => {
    retried = Array.from(pending, (p) => p.key);
    return { pending_count: pending.length };
  };
  w.send({ type: 'storage_flush' });
  await tick();
  assert.deepEqual(
    retried,
    ['b'],
    `hook reported 1 of 2 persisted; next flush should retry 'b', but retried ${JSON.stringify(retried)}; persisted=${JSON.stringify([...persisted.keys()])}; messages=${JSON.stringify(w.posted)}`,
  );
});

test('H4 counter-check: default opfs-flush.js with no OPFS throws and retains (not silent)', async () => {
  const w = boot({ missing: ['fs-access-host.js'] }); // navigator undefined in this context
  w.send({ type: 'storage_put', key: 'doc/1', bytes: [1] });
  w.send({ type: 'storage_flush' });
  await tick();
  let retried = [];
  w.ctx.craftRuntimeFlushPending = async ({ pending }) => {
    retried = Array.from(pending, (p) => p.key);
    return { pending_count: pending.length };
  };
  w.send({ type: 'storage_flush' });
  await tick();
  assert.equal(w.posted[0].type, 'error');
  assert.deepEqual(retried, ['doc/1']);
});

// A hook whose writes complete when the test says so, persisting into `store`.
function gatedHook(store) {
  const calls = [];
  const hook = ({ pending }) =>
    new Promise((resolve, reject) => {
      calls.push({
        pending,
        ok: () => {
          for (const { key, bytes } of pending) store.set(key, bytes);
          resolve({ pending_count: pending.length });
        },
        fail: (e) => reject(new Error(e)),
      });
    });
  return { hook, calls };
}

test('M1: two overlapping flushes settle out of order -> the newer bytes win either way', async () => {
  const store = new Map();
  const g = gatedHook(store);
  const w = boot({ missing: ['fs-access-host.js'], flushHook: g.hook });
  w.send({ type: 'storage_put', key: 'doc', bytes: 'v1' });
  w.send({ type: 'storage_flush' });
  w.send({ type: 'storage_put', key: 'doc', bytes: 'v2' });
  w.send({ type: 'storage_flush' });
  g.calls[1].ok(); // newer flush (v2) finishes first
  await tick();
  g.calls[0].ok(); // older flush (v1) finishes later
  await tick();
  assert.equal(store.get('doc'), 'v2', `latest write was v2 but storage holds ${store.get('doc')}; messages=${JSON.stringify(w.posted)}`);
});

test('M1: older flush fails after newer succeeded -> stale bytes are never restored over it', async () => {
  const store = new Map();
  const g = gatedHook(store);
  const w = boot({ missing: ['fs-access-host.js'], flushHook: g.hook });
  w.send({ type: 'storage_put', key: 'doc', bytes: 'v1' });
  w.send({ type: 'storage_flush' });
  w.send({ type: 'storage_put', key: 'doc', bytes: 'v2' });
  w.send({ type: 'storage_flush' });
  g.calls[1].ok();
  await tick();
  g.calls[0].fail('quota');
  await tick();
  w.send({ type: 'storage_flush' }); // next periodic flush
  await tick();
  g.calls[2]?.ok();
  await tick();
  assert.equal(store.get('doc'), 'v2', `v2 was persisted; the failed v1 flush must not resurrect stale bytes over it: storage holds ${store.get('doc')}`);
});
