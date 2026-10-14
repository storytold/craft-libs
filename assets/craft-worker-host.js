// SharedWorker bootstrap for craft-runtime (craft-libs#5).
// Message `type` tags match craft-worker-host Rust serde (snake_case):
//   Host → worker: init, storage_flush, gpu_jobs, cancel
//   Worker → host: ready, gpu_batch_done, error
const connections = new Set();

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

function handleHostMessage(port, data) {
  if (!data || typeof data.type !== 'string') {
    postError(port, 'missing message type');
    return;
  }

  switch (data.type) {
    case 'init':
      port.postMessage({ type: 'ready', caps: STUB_CAPS });
      return;
    case 'storage_flush':
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

self.onconnect = (event) => {
  const port = event.ports[0];
  connections.add(port);
  port.onmessage = (ev) => handleHostMessage(port, ev.data);
  port.start();
};
