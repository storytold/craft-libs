// Shared message router for craft-worker-host (SharedWorker + dedicated fallback).
// Message `type` tags match craft-worker-host Rust serde (snake_case):
//   Host → worker: init, storage_put, storage_flush, gpu_jobs, cancel
//   Worker → host: ready, gpu_batch_done, storage_flushed, error
//
// WASM/runtime hooks (assign before or after loading WASM in the worker global):
//
//   globalThis.craftRuntimeHandleGpuBatch = async ({ batch_id, jobs }) => {
//     // Return GpuJobOutcome[] aligned with `jobs` (see craft_gpu::GpuJobOutcome).
//   };
//
//   globalThis.craftRuntimeFlushPending = async (data) => {
//     // Persist warm-tier pending writes; return { pending_count: number }.
//     // Default implementation is installed by opfs-flush.js when importScripts succeeds.
//   };

// In-worker pending buffer: key -> live entry { key, bytes } (warm-tier coalescing mirrors
// MirrorStore). storage_put accumulates entries, mutating an already-queued entry's `bytes` in
// place rather than replacing it, so a flush already in flight for that key always resolves
// against the latest write, never a frozen stale snapshot. storage_flush hands the CURRENT
// entries to craftRuntimeFlushPending by reference and removes a key only once a flush confirms
// it was actually persisted: a missing or failing hook leaves every entry exactly where it was,
// so nothing queued is ever silently lost.
const pendingBuffer = new Map();

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

function stubGpuOutcomes(jobs) {
  return jobs.map(() => ({ outputs: [], data: null }));
}

async function dispatchGpuJobs(port, data) {
  const batchId = data.batch_id;
  const jobs = Array.isArray(data.jobs) ? data.jobs : [];
  const hook = globalThis.craftRuntimeHandleGpuBatch;

  try {
    let outcomes;
    if (typeof hook === 'function') {
      outcomes = await hook({ batch_id: batchId, jobs });
    } else {
      outcomes = stubGpuOutcomes(jobs);
    }
    if (!Array.isArray(outcomes)) {
      throw new Error('craftRuntimeHandleGpuBatch must return an outcomes array');
    }
    port.postMessage({
      type: 'gpu_batch_done',
      batch_id: batchId,
      outcomes,
    });
  } catch (err) {
    const message =
      err && typeof err.message === 'string' ? err.message : String(err);
    postError(port, message);
  }
}

async function dispatchStorageFlush(port, _data) {
  // Snapshot the entries currently queued, by reference (not a copy). Do NOT remove them from
  // pendingBuffer yet: that only happens below, once a hook call confirms a key was actually
  // written. Until then, a storage_put for one of these keys mutates the SAME entry object, so
  // any flush already awaiting the hook for it resolves against the latest bytes.
  const batch = Array.from(pendingBuffer.values());
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

  let result;
  try {
    result = await hook({ pending: batch });
  } catch (err) {
    // The hook itself failed: every entry stays queued (nothing was removed above) for retry.
    postError(port, err);
    return;
  }

  // The hook's contract reports only a count, not which keys landed on disk. Entries are
  // confirmed oldest-touched first, matching MirrorStore's coalescing order; an entry past the
  // confirmed count stays queued and is retried on the next flush.
  const confirmed =
    result && typeof result.pending_count === 'number' ? Math.max(0, Math.min(result.pending_count, batch.length)) : 0;
  for (let i = 0; i < confirmed; i += 1) {
    const entry = batch[i];
    if (pendingBuffer.get(entry.key) === entry) {
      pendingBuffer.delete(entry.key);
    }
  }
  port.postMessage({ type: 'storage_flushed', pending_count: confirmed });
}

function handleHostMessage(port, data) {
  if (!data || typeof data.type !== 'string') {
    postError(port, 'missing message type');
    return;
  }

  switch (data.type) {
    case 'init':
      // Record the app id so warm-tier hooks (opfs-flush.js) can namespace storage under it
      // instead of a name shared by every Craft app.
      if (typeof data.app_id === 'string' && data.app_id.length > 0) {
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
    case 'storage_put': {
      if (!data.key) {
        return;
      }
      const key = String(data.key);
      const existing = pendingBuffer.get(key);
      if (existing) {
        // Mutate the already-queued entry in place: a flush already awaiting the hook for this
        // key holds this same object and will see the update when it reads `bytes`.
        existing.bytes = data.bytes;
      } else {
        pendingBuffer.set(key, { key, bytes: data.bytes });
      }
      return;
    }
    case 'storage_flush':
      void dispatchStorageFlush(port, data);
      return;
    case 'gpu_jobs':
      void dispatchGpuJobs(port, data);
      return;
    case 'cancel':
      return;
    default:
      postError(port, `unknown host message type: ${data.type}`);
  }
}

if (typeof globalThis !== 'undefined') {
  globalThis.craftWorkerHostHandleMessage = handleHostMessage;
}
