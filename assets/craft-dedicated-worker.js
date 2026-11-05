// DedicatedWorker fallback for craft-runtime when SharedWorker cannot host WebGPU.
// Same HostMessage / WorkerMessage protocol and hooks as craft-worker-host.js.
// One worker instance per tab; no cross-tab connection tracking needed.

try {
  importScripts('craft-worker-host-routing.js');
} catch {
  // Node protocol smoke tests load routing directly without importScripts.
}

try {
  importScripts('opfs-flush.js');
} catch {
  // Optional warm-tier flush hook.
}

try {
  importScripts('fs-access-host.js');
} catch {
  // Optional cold-tier FS Access host; browser-only.
}

self.onmessage = (event) => {
  // Resolve route lazily per message so hooks assigned after importScripts
  // (e.g. after async WASM init) remain visible -- mirrors SharedWorker's
  // per-connection resolution in craft-worker-host.js.
  const route =
    typeof handleHostMessage === 'function'
      ? handleHostMessage
      : globalThis.craftWorkerHostHandleMessage;
  if (typeof route !== 'function') {
    self.postMessage({
      type: 'error',
      message: 'craft-dedicated-worker: routing not loaded; message dropped',
    });
    return;
  }
  route(self, event.data);
};
