// Re-verification extra for PR #662, adopted from skylartaylor's consolidated review
// (storytold/craft-libs#123, 2026-11-12, gist c1108ecf20d63c5e3183947408ddab7a, file
// 15b-worker-host-662-extra.mjs): a put landing while an earlier flush is still writing must not
// be lost through the SHIPPED default hook (assets/opfs-flush.js), driven here against a fake
// OPFS whose file writes complete only when the test says so, instead of the synthetic test hook
// `review_repro.mjs`'s own M1 tests use.
//
// The gist file needed two fixes to actually exercise that scenario instead of failing on an
// unrelated harness gap: its fake worker context omitted `TextEncoder`, which
// `sanitizeAppSegment` needs and the real browser always provides, and its expected OPFS path
// still used the pre-injective-sanitizer spelling (`collab-photo`) instead of the current
// hex-escaped one (`collab~2ephoto`). With both fixed, the test passes against the current
// `craft-worker-host-routing.js` / `opfs-flush.js`: the revision captured at flush-dispatch time
// (`runStorageFlush`'s `capturedRevisions`) already keeps the ack from clearing a pending entry a
// later `storage_put` rewrote while the flush was in flight. This stands as coverage that the
// finding is addressed, not as a reproduction of a live bug.
//
// The gist's second test in this file asserted two flushes' hook calls running concurrently
// (`closes.length === 2` before either settles). That is no longer possible: `craft-worker-host-
// routing.js` already serializes every `craftRuntimeFlushPending` call through one `flushChain`
// (the earlier "M1 (serialized)" tests in `review_repro.mjs`), so a second flush's hook call now
// never starts until the first settles. That assertion is therefore testing a premise the current
// architecture no longer allows, not a defect, so it is not adopted here.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';

const ASSETS = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../../assets');
const tick = () => new Promise((r) => setTimeout(r, 5));

function fakeOpfs() {
  const files = new Map(); // full path -> bytes (as array)
  const closes = []; // { path, bytes, release }
  const dir = (prefix) => ({
    async getDirectoryHandle(name) {
      if (name.includes('/')) throw new TypeError('Name is not allowed.');
      return dir(prefix + '/' + name);
    },
    async getFileHandle(name) {
      const p = prefix + '/' + name;
      return {
        async createWritable() {
          let staged;
          return {
            async write(payload) { staged = Array.from(payload); },
            close() {
              return new Promise((resolve) => {
                closes.push({ path: p, bytes: staged, release: () => { files.set(p, staged); resolve(); } });
              });
            },
          };
        },
      };
    },
  });
  return { root: dir(''), files, closes };
}

function boot() {
  const opfs = fakeOpfs();
  const posted = [];
  const ctx = { console, setTimeout, Promise, Uint8Array, ArrayBuffer, Map, Array, TextEncoder };
  ctx.self = ctx;
  ctx.globalThis = ctx;
  ctx.navigator = { storage: { getDirectory: async () => opfs.root } };
  ctx.postMessage = (m) => posted.push(m);
  ctx.importScripts = (name) => {
    if (name === 'fs-access-host.js') throw new Error('NetworkError');
    vm.runInContext(fs.readFileSync(path.join(ASSETS, name), 'utf8'), ctx, { filename: name });
  };
  vm.createContext(ctx);
  vm.runInContext(fs.readFileSync(path.join(ASSETS, 'craft-dedicated-worker.js'), 'utf8'), ctx);
  const send = (data) => ctx.onmessage({ data });
  send({ type: 'init', app_id: 'collab.photo', capabilities: [] });
  return { opfs, posted, send };
}

const FILE = '/collab~2ephoto/store/v1/doc';

test('M1 (shipped hook): a write made while an earlier flush is writing is not lost', async () => {
  const w = boot();
  w.send({ type: 'storage_put', key: 'doc', bytes: [1] }); // v1
  w.send({ type: 'storage_flush' });
  await tick(); // the default hook has read v1 and is writing it
  w.send({ type: 'storage_put', key: 'doc', bytes: [2] }); // v2 arrives meanwhile
  w.opfs.closes[0].release(); // v1's write completes
  await tick();
  w.send({ type: 'storage_flush' }); // the next periodic flush must still carry v2
  await tick();
  for (const c of w.opfs.closes.slice(1)) c.release();
  await tick();
  assert.deepEqual(
    w.opfs.files.get(FILE), [2],
    `latest write was v2 but OPFS holds ${JSON.stringify(w.opfs.files.get(FILE))}; writes issued=${w.opfs.closes.length}; messages=${JSON.stringify(w.posted)}`,
  );
});
