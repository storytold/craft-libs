// Shared message router for craft-worker-host (SharedWorker + dedicated fallback).
// Message `type` tags match craft-worker-host Rust serde (snake_case):
//   Host → worker: init, storage_put, storage_delete, storage_flush, gpu_jobs, cancel
//   Worker → host: ready, gpu_batch_done, storage_flushed, error
//
// WASM/runtime hooks (assign before or after loading WASM in the worker global):
//
//   globalThis.craftRuntimeHandleGpuBatch = async ({ batch_id, jobs }) => {
//     // Return GpuJobOutcome[] aligned with `jobs` (see craft_gpu::GpuJobOutcome). Each is
//     // wrapped as a { status: 'done', outcome } entry in gpu_batch_done's `results`, in the
//     // same order, mirroring craft_worker_host::GpuJobResult.
//   };
//
//   globalThis.craftRuntimeFlushPending = async (data) => {
//     // Persist warm-tier pending writes; return { pending_count: number }.
//     // Default implementation is installed by opfs-flush.js when importScripts succeeds.
//     // `data.app_id` identifies which app's partition this batch belongs to.
//   };
//
// Multi-app isolation (RFC-0023 acceptance requirement 3: namespaces separated by app):
// a SharedWorker can be reached by tabs from more than one app (any app hosted from the same
// worker script URL on one origin shares one SharedWorker process). Each port records its own
// app id from its own `init` call, directly on the port object; every pending-write partition,
// and every flush dispatched from that port, is keyed by that port's app id, never by a single
// mutable global. Two apps on the same worker therefore never cross-contaminate each other's
// buffered writes or warm-tier destination.
const UNINITIALIZED_APP_ID = '';

/** appId -> Map<key, entry { key, bytes, deleted, revision }>. */
const appPartitions = new Map();

function partitionFor(appId) {
  const id = typeof appId === 'string' ? appId : UNINITIALIZED_APP_ID;
  let partition = appPartitions.get(id);
  if (!partition) {
    partition = new Map();
    appPartitions.set(id, partition);
  }
  return partition;
}

function portAppId(port) {
  return typeof port.craftAppId === 'string' ? port.craftAppId : UNINITIALIZED_APP_ID;
}

const STUB_CAPS = {
  backend_name: 'worker-host-stub',
  device_name: 'pending-wasm',
  max_storage_buffer_binding_size: 134217728,
  max_buffer_size: 268435456,
  max_texture_dimension_2d: 8192,
};

function postError(port, message) {
  port.postMessage({ type: 'error', message: String(message) });
}

// Batch ids a tab asked to cancel. A `gpu_jobs` dispatch still has no way to abort
// craftRuntimeHandleGpuBatch mid-flight (the hook is a single async call with no abort signal),
// but a cancelled batch's completion must never reach the tab as if it finished normally.
// Cleared once consumed (either suppressed or the batch never arrives), so this never grows
// unbounded across a long worker lifetime.
const cancelledBatches = new Set();

async function dispatchGpuJobs(port, data) {
  const batchId = data.batch_id;
  const jobs = Array.isArray(data.jobs) ? data.jobs : [];
  const hook = globalThis.craftRuntimeHandleGpuBatch;

  if (typeof hook !== 'function') {
    // No runtime hook installed: there is no way to tell an uninitialized runtime from
    // completed GPU work, so this must never report gpu_batch_done with made-up outcomes.
    postError(port, 'craft-worker-host: no GPU batch hook installed; cannot run jobs');
    return;
  }

  try {
    const outcomes = await hook({ batch_id: batchId, jobs });
    if (cancelledBatches.delete(batchId)) {
      // Cancelled while the hook was running: the work already happened (the hook contract has
      // no abort signal to stop it early), but its completion must not reach the tab as if the
      // batch had run to a normal finish.
      return;
    }
    if (!Array.isArray(outcomes)) {
      throw new Error('craftRuntimeHandleGpuBatch must return an outcomes array');
    }
    // Wire shape matches craft_worker_host::GpuJobResult: one tagged entry per job, in input
    // order. This hook's own contract still answers the whole batch in one call (it has no way
    // to report one job done and another failed), so every entry here is 'done'; a hook that
    // throws fails the whole batch through the catch block below instead.
    const results = outcomes.map((outcome) => ({ status: 'done', outcome }));
    port.postMessage({
      type: 'gpu_batch_done',
      batch_id: batchId,
      results,
    });
  } catch (err) {
    cancelledBatches.delete(batchId);
    const message =
      err && typeof err.message === 'string' ? err.message : String(err);
    postError(port, message);
  }
}

// Only one craftRuntimeFlushPending call is ever in flight at a time, across every app
// partition. Two overlapping storage_flush messages used to start independent hook calls with
// no ordering guarantee over the underlying file writes; per the File System standard, a
// writable stream applies its buffer to the file when the stream closes, so an older call
// finishing its file write after a newer one could silently replace the newer bytes.
// Serializing every hook call removes the race by construction: there is never a second
// concurrent write for the same (or any) key to interleave with.
let flushChain = Promise.resolve();

function enqueueFlush(task) {
  const result = flushChain.then(task, task);
  // Keep the chain alive even if this task rejects, so a failed flush does not wedge every
  // flush after it.
  flushChain = result.then(
    () => undefined,
    () => undefined,
  );
  return result;
}

async function runStorageFlush(port, appId) {
  const partition = partitionFor(appId);
  // Snapshot the entries currently queued, by reference (not a copy). Do NOT remove them from
  // the partition yet: that only happens below, once a hook call confirms a key's exact
  // revision was actually written. Until then, a storage_put/storage_delete for one of these
  // keys bumps the SAME entry's revision in place, so a flush already awaiting the hook for it
  // can tell whether the bytes it is about to confirm are still the current ones.
  const batch = Array.from(partition.values());
  if (batch.length === 0) {
    port.postMessage({ type: 'storage_flushed', pending_count: 0 });
    return;
  }

  const hook = globalThis.craftRuntimeFlushPending;
  if (typeof hook !== 'function') {
    // No warm-tier hook installed (e.g. opfs-flush.js failed to load): nothing was persisted.
    // Every entry stays queued for the next flush attempt; report the failure, never a success.
    postError(port, 'craft-worker-host: no storage flush hook installed; writes remain queued');
    return;
  }

  // Capture each entry's revision now, before awaiting the hook: this is the revision the hook
  // is about to persist (or fail to). A storage_put/storage_delete that arrives while the hook
  // call is in flight bumps the entry's revision past this snapshot, so deletion below can tell
  // a write the hook actually saw from one it raced past.
  const capturedRevisions = batch.map((entry) => entry.revision);

  let result;
  try {
    result = await hook({ pending: batch, app_id: appId });
  } catch (err) {
    // The hook itself failed: every entry stays queued (nothing was removed above) for retry.
    postError(port, err);
    return;
  }

  // The hook's contract reports only a count, not which keys landed on disk. Entries are
  // confirmed oldest-touched first, matching MirrorStore's coalescing order; an entry past the
  // confirmed count stays queued and is retried on the next flush. An entry is removed only
  // when it is still the exact revision the hook just persisted: a newer write that raced past
  // it stays queued, so its own, later bytes get their own flush instead of being dropped as
  // "already confirmed" when they never were.
  const confirmed =
    result && typeof result.pending_count === 'number' ? Math.max(0, Math.min(result.pending_count, batch.length)) : 0;
  for (let i = 0; i < confirmed; i += 1) {
    const entry = batch[i];
    if (partition.get(entry.key) === entry && entry.revision === capturedRevisions[i]) {
      partition.delete(entry.key);
    }
  }
  port.postMessage({ type: 'storage_flushed', pending_count: confirmed });
}

function dispatchStorageFlush(port, _data) {
  const appId = portAppId(port);
  void enqueueFlush(() => runStorageFlush(port, appId));
}

function handleHostMessage(port, data) {
  if (!data || typeof data.type !== 'string') {
    postError(port, 'missing message type');
    return;
  }

  switch (data.type) {
    case 'init': {
      // Record the app id on THIS port, not a shared global: a SharedWorker can be reached by
      // more than one app (every app on an origin that loads the same worker script URL shares
      // one worker process), and two ports belonging to different apps must never be able to
      // read or namespace each other's pending writes.
      if (typeof data.app_id === 'string' && data.app_id.length > 0) {
        port.craftAppId = data.app_id;
        // Back-compat: some hooks/tests still read a single shared app id directly.
        globalThis.craftAppId = data.app_id;
      }
      // If the caller transferred a compiled WebAssembly.Module (structured-cloned),
      // forward it to craftRuntimeReceiveWasmModule so the runtime can instantiate
      // without re-fetching or re-compiling. Works for both SharedWorker (shared
      // across tabs) and DedicatedWorker (per-tab module clone).
      if (
        typeof WebAssembly !== 'undefined' &&
        data.wasm_module instanceof WebAssembly.Module
      ) {
        const wasmHook = globalThis.craftRuntimeReceiveWasmModule;
        if (typeof wasmHook === 'function') {
          wasmHook(data.wasm_module);
        }
      }
      port.postMessage({ type: 'ready', caps: STUB_CAPS });
      return;
    }
    case 'storage_put': {
      if (!data.key) {
        return;
      }
      const partition = partitionFor(portAppId(port));
      const key = String(data.key);
      const existing = partition.get(key);
      if (existing) {
        // Mutate the already-queued entry in place, bumping its revision: a flush already
        // awaiting the hook for this key holds this same object and captured the prior
        // revision, so it can tell its confirmation is now stale once this resolves.
        existing.bytes = data.bytes;
        existing.deleted = false;
        existing.revision += 1;
      } else {
        partition.set(key, { key, bytes: data.bytes, deleted: false, revision: 0 });
      }
      return;
    }
    case 'storage_delete': {
      if (!data.key) {
        return;
      }
      const partition = partitionFor(portAppId(port));
      const key = String(data.key);
      const existing = partition.get(key);
      if (existing) {
        existing.bytes = [];
        existing.deleted = true;
        existing.revision += 1;
      } else {
        partition.set(key, { key, bytes: [], deleted: true, revision: 0 });
      }
      return;
    }
    case 'storage_flush':
      dispatchStorageFlush(port, data);
      return;
    case 'gpu_jobs':
      void dispatchGpuJobs(port, data);
      return;
    case 'cancel':
      if (typeof data.batch_id !== 'undefined') {
        cancelledBatches.add(data.batch_id);
      }
      return;
    default:
      postError(port, `unknown host message type: ${data.type}`);
  }
}

if (typeof globalThis !== 'undefined') {
  globalThis.craftWorkerHostHandleMessage = handleHostMessage;
}
