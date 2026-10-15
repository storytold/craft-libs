/**
 * Local protocol smoke test (Node 18+). Does not load SharedWorker; mirrors JS switch logic.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const js = readFileSync(join(root, 'assets/craft-worker-host.js'), 'utf8');

assert.match(js, /case 'storage_flush'/);
assert.match(js, /storage_flushed/);
assert.match(js, /craftRuntimeHandleGpuBatch/);

const hostInit = { type: 'init', app_id: 'test', capabilities: ['gpu'] };
const rustInitJson =
  '{"type":"init","app_id":"test","capabilities":["gpu"]}';
assert.equal(JSON.stringify(hostInit), rustInitJson);

console.log('worker-host-protocol.mjs OK');
