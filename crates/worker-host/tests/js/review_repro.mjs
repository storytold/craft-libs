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

// Boots craft-worker-host-routing.js alone (no SharedWorker/DedicatedWorker wrapper), for tests
// that need to drive `handleHostMessage` with more than one distinct port object directly --
// the thing a real SharedWorker's `onconnect` does once per connecting tab.
function bootRouting({ flushHook, gpuHook } = {}) {
  const ctx = { console, setTimeout, Promise, Uint8Array, ArrayBuffer, Map, Set, Array };
  ctx.self = ctx;
  ctx.globalThis = ctx;
  vm.createContext(ctx);
  vm.runInContext(readAsset('craft-worker-host-routing.js'), ctx);
  if (flushHook) ctx.craftRuntimeFlushPending = flushHook;
  if (gpuHook) ctx.craftRuntimeHandleGpuBatch = gpuHook;
  return ctx;
}

function makePort() {
  const posted = [];
  return { port: { postMessage: (m) => posted.push(m) }, posted };
}

// --- Failure 1: hard-coded, slash-containing OPFS directory -----------------------------------

// Minimal in-memory stand-in for the OPFS directory API, strict about the one rule that matters
// here: a path SEGMENT must never contain '/'. This is exactly what throws
// "TypeError: Failed to execute 'getDirectoryHandle' on 'FileSystemDirectoryHandle': Name is not
// allowed." in real Chrome.
class FakeDir {
  constructor() {
    this.dirs = new Map();
    this.files = new Map();
    // Set by a test to force removeEntry to fail with something other than "not found" (a
    // revoked permission, quota, IO -- anything that must reject the flush rather than be
    // swallowed as "already deleted").
    this.removeEntryError = null;
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
  async removeEntry(name) {
    if (this.removeEntryError) {
      throw this.removeEntryError;
    }
    if (!this.files.delete(name) && !this.dirs.delete(name)) {
      throw new Error('NotFoundError');
    }
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
  const ctx = { console, setTimeout, Uint8Array, ArrayBuffer, TextEncoder };
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
  // getDirectoryHandle, or FakeDir's own check above would have thrown). The '.' in "collab.photo"
  // is hex-escaped (injective encoding, review_repro 2), not collapsed to a hyphen.
  assert.deepEqual([...root.dirs.keys()], ['collab~2ephoto']);
  const appDir = root.dirs.get('collab~2ephoto');
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
  assert.deepEqual([...a.root.dirs.keys()], ['collab~2ephoto']);
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

// review_repro 2 (CodeRabbit finding on craft-worker-host-routing.js:155): flushes are now
// serialized through a single in-flight chain, so these two tests assert the post-fix shape of
// M1's scenario -- a second flush's hook call never starts until the first has fully settled --
// instead of the pre-fix "two concurrent hook calls settle out of order" shape, which the fix
// makes unreachable by construction.

test('M1 (serialized): two sequential flushes settle in order; the second never starts until the first settles', async () => {
  const store = new Map();
  const g = gatedHook(store);
  const w = boot({ missing: ['fs-access-host.js'], flushHook: g.hook });
  w.send({ type: 'storage_put', key: 'doc', bytes: 'v1' });
  w.send({ type: 'storage_flush' });
  await tick();
  assert.equal(g.calls.length, 1, 'the first flush must have started its hook call');

  w.send({ type: 'storage_put', key: 'doc', bytes: 'v2' });
  w.send({ type: 'storage_flush' });
  await tick();
  assert.equal(g.calls.length, 1, 'the second flush must not start its hook call before the first settles');

  g.calls[0].ok(); // first flush (v1) settles
  await tick();
  assert.equal(g.calls.length, 2, 'the second flush must start once the first settles');
  g.calls[1].ok(); // second flush (v2) settles
  await tick();
  assert.equal(store.get('doc'), 'v2', `latest write was v2 but storage holds ${store.get('doc')}; messages=${JSON.stringify(w.posted)}`);
});

test('M1 (serialized): an older queued flush is superseded by newer bytes before its hook call ever starts', async () => {
  const store = new Map();
  const g = gatedHook(store);
  const w = boot({ missing: ['fs-access-host.js'], flushHook: g.hook });
  w.send({ type: 'storage_put', key: 'doc', bytes: 'v1' });
  w.send({ type: 'storage_flush' });
  await tick();

  w.send({ type: 'storage_put', key: 'doc', bytes: 'v2' });
  w.send({ type: 'storage_flush' }); // queued behind the first flush's still-outstanding hook call
  await tick();

  g.calls[0].fail('quota'); // the first flush fails; 'doc' stays queued, already carrying v2's bytes
  await tick();
  assert.equal(g.calls.length, 2, 'the second flush must start once the first settles, even by failing');
  // `g.calls[1].pending` is an array from the vm context's own realm: build a plain outer-realm
  // array from it (via the OUTER Array.from) rather than `.map()`, which inherits the vm
  // realm's Array constructor and would fail deepEqual's cross-realm prototype check even when
  // the contents already match.
  assert.deepEqual(
    Array.from(g.calls[1].pending, (e) => e.bytes),
    ['v2'],
    'the second flush must see the current bytes, never the stale snapshot the first flush captured',
  );
  g.calls[1].ok();
  await tick();
  assert.equal(store.get('doc'), 'v2', `v2 was persisted; storage holds ${store.get('doc')}`);
});

// --- review_repro 2: CodeRabbit findings on PR #662's review (craft-worker-host-routing.js,
// opfs-flush.js, fs-access-host.js) -------------------------------------------------------------

test('multi-app isolation: two app ids on one worker never share a pending partition or a flush batch', async () => {
  const seen = [];
  const ctx = bootRouting({
    flushHook: async ({ pending, app_id }) => {
      seen.push({ app_id, keys: pending.map((p) => p.key) });
      return { pending_count: pending.length };
    },
  });
  const a = makePort();
  const b = makePort();
  ctx.handleHostMessage(a.port, { type: 'init', app_id: 'app-a' });
  ctx.handleHostMessage(b.port, { type: 'init', app_id: 'app-b' });
  ctx.handleHostMessage(a.port, { type: 'storage_put', key: 'config/x', bytes: [1] });
  ctx.handleHostMessage(b.port, { type: 'storage_put', key: 'config/x', bytes: [2] });
  ctx.handleHostMessage(a.port, { type: 'storage_flush' });
  ctx.handleHostMessage(b.port, { type: 'storage_flush' });
  await tick();
  assert.equal(seen.length, 2, `both ports must flush their own batch: ${JSON.stringify(seen)}`);
  const byApp = Object.fromEntries(seen.map((s) => [s.app_id, s.keys]));
  assert.deepEqual(byApp['app-a'], ['config/x'], `app-a's flush must not see app-b's entry: ${JSON.stringify(seen)}`);
  assert.deepEqual(byApp['app-b'], ['config/x'], `app-b's flush must not see app-a's entry: ${JSON.stringify(seen)}`);
});

// CodeRabbit finding on routing.js:221: a storage_put sent before a port's own init lands in
// the '' (uninitialized) partition. Before the fix, a later storage_flush from that same,
// now-initialized port selected the real app partition instead, so the pre-init write was
// stranded in '' with nothing ever flushing it again.
test('a storage_put sent before init is not lost once init arrives', async () => {
  const seen = [];
  const ctx = bootRouting({
    flushHook: async ({ pending, app_id }) => {
      seen.push({ app_id, keys: pending.map((p) => p.key) });
      return { pending_count: pending.length };
    },
  });
  const p = makePort();
  ctx.handleHostMessage(p.port, { type: 'storage_put', key: 'config/early', bytes: [1] });
  ctx.handleHostMessage(p.port, { type: 'init', app_id: 'app-a' });
  ctx.handleHostMessage(p.port, { type: 'storage_flush' });
  await tick();

  assert.equal(seen.length, 1, `the pre-init write must still be flushed once init arrives: ${JSON.stringify(seen)}`);
  assert.equal(seen[0].app_id, 'app-a');
  assert.deepEqual(seen[0].keys, ['config/early']);
});

// Two different, not-yet-initialized ports buffer their pre-init writes independently: one
// port's early write must not leak into whichever app the OTHER port initializes as.
test('pre-init writes from two different ports never cross into the wrong app on init', async () => {
  const seen = [];
  const ctx = bootRouting({
    flushHook: async ({ pending, app_id }) => {
      seen.push({ app_id, keys: pending.map((p) => p.key) });
      return { pending_count: pending.length };
    },
  });
  const a = makePort();
  const b = makePort();
  ctx.handleHostMessage(a.port, { type: 'storage_put', key: 'config/from-a', bytes: [1] });
  ctx.handleHostMessage(b.port, { type: 'storage_put', key: 'config/from-b', bytes: [2] });
  ctx.handleHostMessage(a.port, { type: 'init', app_id: 'app-a' });
  ctx.handleHostMessage(b.port, { type: 'init', app_id: 'app-b' });
  ctx.handleHostMessage(a.port, { type: 'storage_flush' });
  ctx.handleHostMessage(b.port, { type: 'storage_flush' });
  await tick();

  const byApp = Object.fromEntries(seen.map((s) => [s.app_id, s.keys]));
  assert.deepEqual(byApp['app-a'], ['config/from-a']);
  assert.deepEqual(byApp['app-b'], ['config/from-b']);
});

test('missing craftRuntimeHandleGpuBatch reports error, never a fabricated gpu_batch_done', async () => {
  const ctx = bootRouting();
  const p = makePort();
  ctx.handleHostMessage(p.port, { type: 'gpu_jobs', batch_id: 1, jobs: [{ provider_id: 'x', params: [], inputs: [], outputs: [] }] });
  await tick();
  assert.equal(p.posted.length, 1);
  assert.equal(p.posted[0].type, 'error', `a missing hook must never report success: ${JSON.stringify(p.posted)}`);
});

// PR 663: ProviderRegistry::dispatch_batch and craft_worker_host::GpuJobResult moved to a
// per-job result, in input order, instead of one outcomes array for the whole batch.
// gpu_batch_done's wire shape follows: `results`, each tagged { status: 'done', outcome } here
// (this hook's own contract still answers a batch in one call, so every entry is 'done').
test('gpu_batch_done wraps the hook outcomes as per-job results, in order', async () => {
  const ctx = bootRouting({
    gpuHook: async ({ jobs }) => jobs.map(() => ({ outputs: [], data: null })),
  });
  const p = makePort();
  ctx.handleHostMessage(p.port, {
    type: 'gpu_jobs',
    batch_id: 9,
    jobs: [
      { provider_id: 'x', params: [], inputs: [], outputs: [] },
      { provider_id: 'x', params: [], inputs: [], outputs: [] },
    ],
  });
  await tick();
  assert.equal(p.posted.length, 1);
  const [msg] = p.posted;
  assert.equal(msg.type, 'gpu_batch_done');
  assert.equal(msg.results.length, 2);
  // `msg.results` is a vm-realm array (built inside routing.js); rebuild the mapped array
  // through the OUTER Array.from before comparing, same reasoning as the M1 fix above.
  assert.deepEqual(
    Array.from(msg.results, (r) => r.status),
    ['done', 'done'],
  );
});

test('a write that changes mid-flush is not dropped as confirmed by a stale success', async () => {
  let resolveFirstHook;
  const ctx = bootRouting({
    flushHook: ({ pending }) => new Promise((resolve) => {
      resolveFirstHook = () => resolve({ pending_count: pending.length });
    }),
  });
  const p = makePort();
  ctx.handleHostMessage(p.port, { type: 'storage_put', key: 'doc', bytes: [1] });
  ctx.handleHostMessage(p.port, { type: 'storage_flush' }); // captures doc's revision before awaiting the hook
  await tick();
  // A newer write lands while that flush's hook call is still outstanding.
  ctx.handleHostMessage(p.port, { type: 'storage_put', key: 'doc', bytes: [2] });
  resolveFirstHook(); // the stale hook call now reports success for the OLD revision
  await tick();

  let retriedBytes = null;
  ctx.craftRuntimeFlushPending = async ({ pending }) => {
    retriedBytes = pending.map((e) => e.bytes);
    return { pending_count: pending.length };
  };
  ctx.handleHostMessage(p.port, { type: 'storage_flush' });
  await tick();
  assert.deepEqual(retriedBytes, [[2]], `the newer bytes must still be queued, not dropped as already confirmed: ${JSON.stringify(retriedBytes)}`);
});

test('overlapping flushes are serialized: a second hook call never starts until the first settles', async () => {
  const order = [];
  let releaseFirst;
  const ctx = bootRouting({
    flushHook: async ({ pending }) => {
      const key = pending[0].key;
      order.push(`start:${key}`);
      if (key === 'first') {
        await new Promise((resolve) => {
          releaseFirst = resolve;
        });
      }
      order.push(`end:${key}`);
      return { pending_count: pending.length };
    },
  });
  const p = makePort();
  ctx.handleHostMessage(p.port, { type: 'storage_put', key: 'first', bytes: [1] });
  ctx.handleHostMessage(p.port, { type: 'storage_flush' });
  await tick();
  ctx.handleHostMessage(p.port, { type: 'storage_put', key: 'second', bytes: [2] });
  ctx.handleHostMessage(p.port, { type: 'storage_flush' });
  await tick();
  assert.deepEqual(order, ['start:first'], `the second flush's hook call must not start before the first settles: ${JSON.stringify(order)}`);
  releaseFirst();
  await tick();
  assert.deepEqual(order, ['start:first', 'end:first', 'start:second', 'end:second']);
});

test('cancel suppresses gpu_batch_done for that batch even though the hook already ran', async () => {
  let resolveHook;
  const ctx = bootRouting({ gpuHook: () => new Promise((resolve) => { resolveHook = () => resolve([{ outputs: [], data: null }]); }) });
  const p = makePort();
  ctx.handleHostMessage(p.port, { type: 'gpu_jobs', batch_id: 5, jobs: [{ provider_id: 'x', params: [], inputs: [], outputs: [] }] });
  await tick();
  ctx.handleHostMessage(p.port, { type: 'cancel', batch_id: 5 });
  resolveHook();
  await tick();
  assert.equal(p.posted.length, 0, `a cancelled batch must never post gpu_batch_done: ${JSON.stringify(p.posted)}`);
});

// CodeRabbit finding on routing.js:63: cancellation was tracked in one Set shared by every
// port, keyed only by batch_id. Nothing makes batch_id globally unique, so a cancellation from
// one port could consume the shared entry and suppress a different port's completion for what
// is, to that port, an unrelated batch.
test('cancel is scoped to the requesting port: a batch_id collision on another port is not suppressed', async () => {
  const resolvers = [];
  const ctx = bootRouting({
    gpuHook: () =>
      new Promise((resolve) => {
        resolvers.push(resolve);
      }),
  });
  const a = makePort();
  const b = makePort();
  // Both ports happen to pick the same batch_id; nothing in the protocol makes it unique.
  ctx.handleHostMessage(a.port, { type: 'gpu_jobs', batch_id: 9, jobs: [{ provider_id: 'x', params: [], inputs: [], outputs: [] }] });
  ctx.handleHostMessage(b.port, { type: 'gpu_jobs', batch_id: 9, jobs: [{ provider_id: 'x', params: [], inputs: [], outputs: [] }] });
  await tick();
  assert.equal(resolvers.length, 2);

  ctx.handleHostMessage(a.port, { type: 'cancel', batch_id: 9 }); // A cancels its OWN batch 9
  resolvers[0]([{ outputs: [], data: null }]);
  resolvers[1]([{ outputs: [], data: null }]);
  await tick();

  assert.equal(a.posted.length, 0, 'A cancelled its own batch: no completion must reach it');
  assert.equal(b.posted.length, 1, `B never cancelled anything: its completion for the same batch_id must still arrive, got ${JSON.stringify(b.posted)}`);
  assert.equal(b.posted[0].type, 'gpu_batch_done');
});

test('sanitizeAppSegment: distinct app ids never collapse to the same OPFS root segment', async () => {
  const ctx = {};
  ctx.globalThis = ctx;
  ctx.TextEncoder = TextEncoder;
  vm.createContext(ctx);
  vm.runInContext(readAsset('opfs-flush.js'), ctx);
  const a = ctx.craftOpfsWarmRootSegments('collab.photo')[0];
  const b = ctx.craftOpfsWarmRootSegments('collab-photo')[0];
  assert.notEqual(a, b, `collab.photo and collab-photo must not collapse to the same segment, both were ${JSON.stringify(a)}`);
  assert.ok(!a.includes('/') && !b.includes('/'));
});

test('OPFS: storage_delete removes a previously-written file via opfs-flush.js', async () => {
  const { ctx, root } = bootOpfs({ appId: 'collabphoto' });
  await ctx.craftRuntimeFlushPending({ pending: [{ key: 'docs/a.bin', bytes: [1, 2, 3], deleted: false }], app_id: 'collabphoto' });
  const appDir = root.dirs.get('collabphoto');
  const docsDir = appDir.dirs.get('store').dirs.get('v1').dirs.get('docs');
  assert.ok(docsDir.files.has('a.bin'), 'the file must exist before the delete');

  const result = await ctx.craftRuntimeFlushPending({ pending: [{ key: 'docs/a.bin', bytes: [], deleted: true }], app_id: 'collabphoto' });
  assert.equal(result.pending_count, 1);
  assert.ok(!docsDir.files.has('a.bin'), 'a deleted key must remove its warm-tier file');
});

test('OPFS: deleting a key that was never written is a no-op, not an error', async () => {
  const { ctx } = bootOpfs({ appId: 'collabphoto' });
  const result = await ctx.craftRuntimeFlushPending({ pending: [{ key: 'docs/never-existed.bin', bytes: [], deleted: true }], app_id: 'collabphoto' });
  assert.equal(result.pending_count, 1);
});

// CodeRabbit finding on opfs-flush.js:105: removeEntry's catch (and the parent-directory catch
// above it) used to swallow every error, not just "not found". A real failure -- a revoked
// permission, quota, IO -- was treated as a successful delete: the flush hook returned
// pending_count confirming it, the router dropped the tombstone, and the file stayed on disk.
test('OPFS: a removeEntry failure other than not-found rejects the flush and leaves the file', async () => {
  const { ctx, root } = bootOpfs({ appId: 'collabphoto' });
  await ctx.craftRuntimeFlushPending({ pending: [{ key: 'docs/a.bin', bytes: [1, 2, 3], deleted: false }], app_id: 'collabphoto' });
  const docsDir = root.dirs.get('collabphoto').dirs.get('store').dirs.get('v1').dirs.get('docs');
  assert.ok(docsDir.files.has('a.bin'), 'the file must exist before the delete attempt');

  docsDir.removeEntryError = new Error('QuotaExceededError: cannot modify this entry right now');
  await assert.rejects(
    () => ctx.craftRuntimeFlushPending({ pending: [{ key: 'docs/a.bin', bytes: [], deleted: true }], app_id: 'collabphoto' }),
    /QuotaExceededError/,
    'a non-not-found removeEntry failure must reject the flush, not be swallowed as a successful delete',
  );
  assert.ok(docsDir.files.has('a.bin'), 'the file must still exist: the delete never actually succeeded');
});

// --- fs-access-host.js: bounded cold-file reads, honest flush acknowledgment ------------------

function bootFsAccess() {
  const ctx = { console, Uint8Array, ArrayBuffer };
  ctx.globalThis = ctx;
  ctx.FileSystemDirectoryHandle = class {};
  vm.createContext(ctx);
  vm.runInContext(readAsset('fs-access-host.js'), ctx);
  return ctx;
}

class FakeColdFile {
  constructor(size) {
    this.size = size;
  }
  async arrayBuffer() {
    return new ArrayBuffer(this.size);
  }
}
class FakeColdFileHandle {
  constructor(size) {
    this.file = new FakeColdFile(size);
  }
  async getFile() {
    return this.file;
  }
}
class FakeColdDir {
  constructor() {
    this.files = new Map();
    this.dirs = new Map();
  }
  async getDirectoryHandle(name) {
    if (!this.dirs.has(name)) this.dirs.set(name, new FakeColdDir());
    return this.dirs.get(name);
  }
  async getFileHandle(name) {
    if (!this.files.has(name)) throw new Error('NotFoundError');
    return this.files.get(name);
  }
}

// Builds a FakeColdDir tree holding one file at `path` (the segments
// `fsAccessKeySegments` produces, e.g. ['volumes', 'vol-1', 'files', 'big.bin']).
function mkColdTreeWithFile(path, size) {
  const root = new FakeColdDir();
  let dir = root;
  for (const segment of path.slice(0, -1)) {
    const next = new FakeColdDir();
    dir.dirs.set(segment, next);
    dir = next;
  }
  dir.files.set(path[path.length - 1], new FakeColdFileHandle(size));
  return root;
}

test('craftFsAccessReadKey refuses a file over the read budget before allocating it', async () => {
  const ctx = bootFsAccess();
  const root = mkColdTreeWithFile(['volumes', 'vol-1', 'files', 'big.bin'], 1024);
  await assert.rejects(
    () => ctx.craftFsAccessReadKey(root, 'vol-1', 'files/big.bin', 100),
    /over the 100-byte read budget/,
    'a file past the explicit budget must be refused before arrayBuffer() runs',
  );
});

test('craftFsAccessReadKey proceeds for a file within the read budget', async () => {
  const ctx = bootFsAccess();
  const root = mkColdTreeWithFile(['volumes', 'vol-1', 'files', 'small.bin'], 10);
  const bytes = await ctx.craftFsAccessReadKey(root, 'vol-1', 'files/small.bin', 100);
  assert.equal(bytes.length, 10);
});

test('craftFsAccessFlushPending never acknowledges writes it did not persist (no mounted handle)', async () => {
  const ctx = bootFsAccess();
  const result = await ctx.craftFsAccessFlushPending('unmounted-vol', [{ key: 'files/a', bytes: [1, 2, 3] }]);
  assert.equal(result.pending_count, 0, `nothing was written, so pending_count must be 0, got ${JSON.stringify(result)}`);
});
