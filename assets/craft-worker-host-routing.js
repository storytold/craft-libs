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

async function dispatchStorageFlush(port, data) {
  const hook = globalThis.craftRuntimeFlushPending;
  let pendingCount = 0;
  if (typeof hook === 'function') {
    try {
      const result = await hook(data);
      pendingCount =
        result && typeof result.pending_count === 'number'
          ? result.pending_count
          : 0;
    } catch (err) {
      postError(port, err);
      return;
    }
  }
  port.postMessage({ type: 'storage_flushed', pending_count: pendingCount });
}

function handleHostMessage(port, data) {
  if (!data || typeof data.type !== 'string') {
    postError(port, 'missing message type');
    return;
  }

  switch (data.type) {
    case 'init':
      port.postMessage({ type: 'ready', caps: STUB_CAPS });
      return;
    case 'storage_put':
      return;
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
