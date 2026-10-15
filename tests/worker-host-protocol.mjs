/**
 * Local protocol smoke test (Node 18+). Does not load SharedWorker; executes routing JS.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const routingPath = join(root, 'assets/craft-worker-host-routing.js');
const sharedWorkerPath = join(root, 'assets/craft-worker-host.js');
const dedicatedPath = join(root, 'assets/craft-dedicated-worker.js');

const routingJs = readFileSync(routingPath, 'utf8');
const sharedWorkerJs = readFileSync(sharedWorkerPath, 'utf8');
const dedicatedJs = readFileSync(dedicatedPath, 'utf8');

assert.match(sharedWorkerJs, /craft-worker-host-routing\.js/);
assert.match(dedicatedJs, /craft-worker-host-routing\.js/);
assert.match(routingJs, /case 'storage_put'/);
assert.match(routingJs, /case 'storage_flush'/);
assert.match(routingJs, /case 'gpu_jobs'/);
assert.match(routingJs, /case 'cancel'/);
assert.match(routingJs, /craftRuntimeHandleGpuBatch/);
assert.match(routingJs, /craftRuntimeFlushPending/);

function loadRouting(extraGlobals = {}) {
  const messages = [];
  const port = {
    postMessage(msg) {
      messages.push(msg);
    },
  };
  const sandbox = {
    console,
    globalThis: {},
  };
  sandbox.globalThis = sandbox;
  Object.assign(sandbox.globalThis, extraGlobals);
  vm.createContext(sandbox);
  vm.runInContext(routingJs, sandbox);
  const handle =
    sandbox.globalThis.craftWorkerHostHandleMessage ??
    sandbox.handleHostMessage;
  assert.equal(typeof handle, 'function', 'handleHostMessage export');
  return { handle, port, messages };
}

const hostInit = { type: 'init', app_id: 'test', capabilities: ['gpu'] };
const rustInitJson =
  '{"type":"init","app_id":"test","capabilities":["gpu"]}';
assert.equal(JSON.stringify(hostInit), rustInitJson);

const storagePut = {
  type: 'storage_put',
  key: 'config/prefs.json',
  bytes: [1, 2],
};
assert.equal(
  JSON.stringify(storagePut),
  '{"type":"storage_put","key":"config/prefs.json","bytes":[1,2]}',
);

const storageFlush = { type: 'storage_flush' };
assert.equal(JSON.stringify(storageFlush), '{"type":"storage_flush"}');

const gpuJobs = {
  type: 'gpu_jobs',
  batch_id: 42,
  jobs: [
    {
      provider_id: 'craft.test',
      params: [1, 2],
      inputs: [],
      outputs: [],
    },
  ],
};
assert.equal(
  JSON.stringify(gpuJobs),
  '{"type":"gpu_jobs","batch_id":42,"jobs":[{"provider_id":"craft.test","params":[1,2],"inputs":[],"outputs":[]}]}',
);

const cancel = { type: 'cancel', batch_id: 99 };
assert.equal(JSON.stringify(cancel), '{"type":"cancel","batch_id":99}');

{
  const { handle, port, messages } = loadRouting();
  handle(port, hostInit);
  assert.equal(messages.length, 1);
  assert.equal(messages[0].type, 'ready');
  assert.ok(messages[0].caps);
}

{
  const { handle, port, messages } = loadRouting();
  handle(port, storagePut);
  assert.equal(messages.length, 0);
}

{
  const { handle, port, messages } = loadRouting({
    craftRuntimeFlushPending: () => ({ pending_count: 3 }),
  });
  handle(port, storageFlush);
  await new Promise((r) => setImmediate(r));
  assert.equal(messages.length, 1);
  assert.equal(messages[0].type, 'storage_flushed');
  assert.equal(messages[0].pending_count, 3);
}

{
  const { handle, port, messages } = loadRouting({
    craftRuntimeHandleGpuBatch: async ({ batch_id, jobs }) =>
      jobs.map(() => ({ outputs: [], data: null })),
  });
  handle(port, gpuJobs);
  await new Promise((r) => setImmediate(r));
  assert.equal(messages.length, 1);
  assert.equal(messages[0].type, 'gpu_batch_done');
  assert.equal(messages[0].batch_id, 42);
  assert.equal(messages[0].outcomes.length, 1);
}

{
  const { handle, port, messages } = loadRouting();
  handle(port, cancel);
  assert.equal(messages.length, 0);
}

// -- storage_put buffering + flush drains buffer --
{
  const flushed = [];
  const { handle, port, messages } = loadRouting({
    craftRuntimeFlushPending: async ({ pending }) => {
      flushed.push(...pending);
      return { pending_count: pending.length };
    },
  });
  handle(port, storagePut);
  handle(port, { type: 'storage_put', key: 'files/vid.mp4', bytes: [9, 8, 7] });
  // Before flush: no storage_flushed messages yet.
  assert.equal(messages.length, 0);

  handle(port, storageFlush);
  await new Promise((r) => setImmediate(r));

  assert.equal(messages.length, 1);
  assert.equal(messages[0].type, 'storage_flushed');
  assert.equal(messages[0].pending_count, 2);
  assert.equal(flushed.length, 2);
  assert.equal(flushed[0].key, 'config/prefs.json');
  assert.equal(flushed[1].key, 'files/vid.mp4');
}

// -- Duplicate storage_put coalesces (latest bytes win) --
{
  const flushed = [];
  const { handle, port, messages } = loadRouting({
    craftRuntimeFlushPending: async ({ pending }) => {
      flushed.push(...pending);
      return { pending_count: pending.length };
    },
  });
  handle(port, { type: 'storage_put', key: 'config/prefs.json', bytes: [1] });
  handle(port, { type: 'storage_put', key: 'config/prefs.json', bytes: [2, 2] });
  handle(port, storageFlush);
  await new Promise((r) => setImmediate(r));

  assert.equal(flushed.length, 1, 'coalesced to one entry');
  assert.deepEqual(Array.from(flushed[0].bytes), [2, 2], 'latest bytes kept');
  assert.equal(messages[0].pending_count, 1);
}

// -- Second flush after buffer drained is empty --
{
  const callLog = [];
  const { handle, port, messages } = loadRouting({
    craftRuntimeFlushPending: async ({ pending }) => {
      callLog.push(pending.length);
      return { pending_count: pending.length };
    },
  });
  handle(port, storagePut);
  handle(port, storageFlush);
  await new Promise((r) => setImmediate(r));
  handle(port, storageFlush);
  await new Promise((r) => setImmediate(r));

  assert.equal(callLog[0], 1, 'first flush: 1 entry');
  assert.equal(callLog[1], 0, 'second flush: buffer empty');
}

// -- storage_flush with no hook still posts storage_flushed --
{
  const { handle, port, messages } = loadRouting();
  handle(port, storagePut);
  handle(port, storageFlush);
  await new Promise((r) => setImmediate(r));
  assert.equal(messages.length, 1);
  assert.equal(messages[0].type, 'storage_flushed');
  assert.equal(messages[0].pending_count, 1);
}

console.log('worker-host-protocol.mjs OK');
